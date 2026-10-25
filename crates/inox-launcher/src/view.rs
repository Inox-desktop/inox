//! The Qt-free projection QML renders.
//!
//! This file is what `view.rs` was: it makes no decisions of its own, it
//! *resolves* the ones [`crate::view_model`] already made into the concrete
//! values a QML tree binds to — colours as `#AARRGGBB` strings, distances and
//! font sizes as numbers, and every user-visible sentence as the text the
//! catalogs produced. QML then has nothing left to decide: it walks `tiles` and
//! draws them, and draws the chips and the grid geometry the same document
//! carries.
//!
//! The shape is one JSON document (docs/qml-migration-contract.md §3). The whole
//! document is re-sent on every change; that is the accepted trade of this
//! migration — correctness and a clean boundary first, incremental updates once
//! there is data saying they are needed.
//!
//! ```text
//! { "title", "query", "placeholder", "hint", "heading", "notice",
//!   "selected", "category",
//!   "tiles": [ { …, "icon_url", "style": { … } } ],
//!   "chips": [ { "id", "label", "selected", "style": { … } } ],
//!   "grid": { "columns", "cell_width", "cell_height", "gap", "image",
//!             "label_lines" },
//!   "banner": { "kind", …, "action": { "label", "type" } } | null,
//!   "status": { "line", "reason", "color", "retry" } | null,
//!   "flags": { "can_retry_index", "can_retry_launch" },
//!   "focus": { "field", "serial" },
//!   "window": { "width", "height" },
//!   "theme": { "name", "colors", "font", "metrics", "styles", "motion" } }
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
use crate::view_model::{Banner, BannerKind, Chip, Status, Tile, ViewModel};

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
        // The surface title in the top row, beside the search field. It belongs
        // to the window, so it stays while a banner replaces the grid.
        "heading": model.heading,
        "notice": model.notice,
        // The selected tile's position, or `null` when nothing is selected. The
        // grid scrolls to it; the index is the projection's decision, not QML's
        // (`view_model::ViewModel::selected_index`).
        "selected": model.selected_index(),
        // The category the browse is filtered by. Exactly one chip carries
        // `selected: true`, and this is that chip's id — the action channel sends
        // it back when another chip is chosen.
        "category": model.chips.iter().find(|chip| chip.selected).map(|chip| chip.id),
        "tiles": tiles(model, tokens),
        "chips": chips(model, tokens),
        "grid": grid_json(placement),
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

    // The field is one title line plus the control padding and its hairline
    // border, the same sum the placement sizes the top row with.
    let field = theme::line_height(tokens, TextRole::Title)
        + metrics.space_sm * 2.0
        + metrics.hairline * 2.0;

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
            "field_height": px(field),
            // The line box of a caption: the hint strip is drawn at this height
            // rather than at Qt's own line box, so the surface the placement
            // computed and the surface QML lays out are the same height
            // (acceptance Q02: nothing is clipped at 200 %).
            "caption_line_height": px(theme::line_height(tokens, TextRole::Caption)),
            // Every button in this surface — the banner's one action and the
            // status bar's retry — is this tall (guidelines §4: at least
            // `spacing.xl`, growing with the type).
            "action_height": px(theme::action_height(tokens)),
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
        // The token durations, in milliseconds. QML picks *where* motion helps
        // (a hovered row, a state change, the window opening); it never picks a
        // duration of its own (guidelines §8 — the only durations a surface may
        // use are these three).
        "motion": {
            "fast": tokens.motion.fast_ms,
            "normal": tokens.motion.normal_ms,
            "slow": tokens.motion.slow_ms,
        },
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

/// The grid's geometry, as QML draws it.
///
/// The numbers come from [`crate::placement::Placement::grid`], which is also
/// where the keyboard's row step comes from: the tiles a person sees and the
/// tiles an arrow key moves by are the same count.
fn grid_json(placement: &Placement) -> Value {
    let grid = &placement.grid;

    json!({
        "columns": grid.columns,
        "cell_width": px(grid.cell_width),
        "cell_height": px(grid.cell_height),
        "gap": px(grid.gap),
        "image": px(grid.image),
        "label_lines": crate::theme::TILE_LABEL_LINES,
    })
}

/// The application tiles, each with the colours its state resolves to.
fn tiles(model: &ViewModel, tokens: &Theme) -> Vec<Value> {
    let metrics = Metrics::from_tokens(tokens);
    model
        .tiles
        .iter()
        .map(|tile| {
            let mut value = tile_json(tile);
            if let Value::Object(object) = &mut value {
                object.insert("style".to_owned(), tile_style(tile, tokens, &metrics));
            }
            value
        })
        .collect()
}

/// One tile's text, icon and flags.
fn tile_json(tile: &Tile) -> Value {
    json!({
        "id": tile.id,
        "icon": tile.icon,
        "icon_url": tile.icon_url,
        "label": tile.label,
        "truncated": tile.truncated,
        "detail": tile.detail,
        "selected": tile.selected,
        "enabled": tile.enabled,
        "disabled_reason": tile.disabled_reason,
    })
}

/// The category chips, each with the colours its state resolves to.
fn chips(model: &ViewModel, tokens: &Theme) -> Vec<Value> {
    let metrics = Metrics::from_tokens(tokens);
    model
        .chips
        .iter()
        .map(|chip| {
            let mut value = json!({
                "id": chip.id,
                "label": chip.label,
                "selected": chip.selected,
            });
            if let Value::Object(object) = &mut value {
                object.insert("style".to_owned(), chip_style(chip, tokens, &metrics));
            }
            value
        })
        .collect()
}

/// The colours a chip is drawn with.
///
/// The same five states a row has (§5): resting is transparent with `text`,
/// hovering takes the alternate surface, pressing fills with the accent, and the
/// selected chip is the accent fill itself — with `on_accent` on it, because a
/// chip that keeps `text` on `accent` is unreadable. A selected chip does not
/// change its fill under the pointer: there is no hover variant of `accent`, and
/// swapping it for `surface_variant` would read as disabled.
fn chip_style(chip: &Chip, tokens: &Theme, metrics: &Metrics) -> Value {
    let colors = &tokens.colors;
    let accent = theme::qml_color(colors.accent);

    json!({
        "background": if chip.selected { accent.clone() } else { transparent() },
        "background_hover": if chip.selected { accent.clone() } else { theme::qml_color(colors.surface_variant) },
        "background_pressed": accent.clone(),
        "text": if chip.selected { theme::qml_color(colors.on_accent) } else { theme::qml_color(colors.text) },
        "text_pressed": theme::qml_color(colors.on_accent),
        "border": if chip.selected { accent } else { theme::qml_color(colors.border) },
        "border_width": px(metrics.hairline),
        "radius": px(metrics.radius_sm),
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
fn tile_style(row: &Tile, tokens: &Theme, metrics: &Metrics) -> Value {
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
    use crate::categories::Category;
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
        category: Category,
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
                category: Category::All,
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
                category: self.category,
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
    fn the_tiles_follow_the_search_result_and_mark_the_selection() {
        let mut fixture = fixture("en-US");
        fixture.query = "fi".to_owned();
        fixture.selected = 0;
        let value = fixture.project_with(&[("firefox.desktop", "Firefox")]);

        let tiles = value["tiles"].as_array().expect("a tile list");
        assert_eq!(tiles.len(), 1);
        assert_eq!(tiles[0]["id"].as_str(), Some("firefox.desktop"));
        assert_eq!(tiles[0]["label"].as_str(), Some("Firefox"));
        assert_eq!(tiles[0]["selected"].as_bool(), Some(true));
        assert_eq!(tiles[0]["enabled"].as_bool(), Some(true));
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
    }

    #[test]
    fn the_selected_row_is_drawn_in_the_accent_colour() {
        let mut fixture = fixture("en-US");
        fixture.selected = 0;
        let value = fixture.project_with(&[("firefox.desktop", "Firefox")]);
        let style = &value["tiles"][0]["style"];

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
    fn the_tile_states_are_drawn_apart() {
        // The guidelines' §5 states, as this surface draws a tile: resting,
        // hovered, pressed, selected and unusable. `Fixture` selects the first
        // row by default, so "resting" is the fixture that selects nothing.
        let mut plain = fixture("en-US");
        plain.selected = usize::MAX;
        let resting = plain.project_with(&[("editor.desktop", "Text Editor")]);
        let mut selected = fixture("en-US");
        selected.selected = 0;
        let highlighted = selected.project_with(&[("editor.desktop", "Text Editor")]);

        // A tile that cannot be launched stays muted and says why.
        let unusable = {
            let mut entry = AppIndex::test_entry("htop.desktop", "htop", "htop");
            entry.terminal = true;
            Fixture::new("en-US", vec![entry]).project_with(&[("htop.desktop", "htop")])
        };

        // A tile that names an icon draws the icon; one that names none draws the
        // placeholder. `AppIndex::test_entry` names no icon, so the icon row
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

        let resting_style = &resting["tiles"][0]["style"];
        let selected_style = &highlighted["tiles"][0]["style"];
        let unusable_style = &unusable["tiles"][0]["style"];
        let bare_style = &placeholder_row["tiles"][0]["style"];
        let named_style = &icon_row["tiles"][0]["style"];

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
    fn an_unusable_tile_is_muted_and_carries_its_reason() {
        let mut entry = AppIndex::test_entry("htop.desktop", "htop", "htop");
        entry.terminal = true;
        entry.icon = Some("utilities-terminal".to_owned());
        let mut fixture = Fixture::new("en-US", vec![entry]);
        let value = fixture.project_with(&[("htop.desktop", "htop")]);

        let tile = &value["tiles"][0];
        assert_eq!(tile["enabled"].as_bool(), Some(false));
        assert!(
            tile["disabled_reason"]
                .as_str()
                .is_some_and(|reason| reason.contains(
                    &fixture
                        .localizer
                        .message("launcher-failure-missing-terminal")
                )),
            "{:?}",
            tile["disabled_reason"],
        );
        assert_eq!(
            tile["style"]["text"].as_str(),
            Some(theme::qml_color(fixture.tokens.colors.text_muted).as_str()),
            "a row that cannot be launched must not look like the selection",
        );
        assert_ne!(
            tile["style"]["background"].as_str(),
            Some(theme::qml_color(fixture.tokens.colors.accent).as_str())
        );

        // The same tile, selected: still muted and still not filled.
        fixture.selected = 0;
        let selected = fixture.project_with(&[("htop.desktop", "htop")]);
        assert_eq!(
            selected["tiles"][0]["style"]["text"].as_str(),
            Some(theme::qml_color(fixture.tokens.colors.text_muted).as_str())
        );
        assert_ne!(
            selected["tiles"][0]["style"]["background"].as_str(),
            Some(theme::qml_color(fixture.tokens.colors.accent).as_str())
        );
    }

    #[test]
    fn an_empty_query_lists_every_application_without_a_browsing_line() {
        let fixture = fixture("en-US");
        let value = fixture.project_with(&[
            ("editor.desktop", "Text Editor"),
            ("firefox.desktop", "Firefox"),
        ]);

        assert_eq!(value["tiles"].as_array().map(Vec::len), Some(2));
        assert_eq!(value["banner"], Value::Null);
        assert!(
            value.get("prompt").is_none(),
            "the card is the grid: the placeholder says what the field is for",
        );
        assert_eq!(
            value["hint"].as_str(),
            Some(fixture.localizer.message("launcher-search-hint").as_str()),
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

        assert_eq!(value["tiles"].as_array().map(Vec::len), Some(0));
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
    fn the_hint_offers_only_the_keys_the_screen_binds() {
        // With rows, Up/Down and Enter are bound, so the hint names them.
        let plain = fixture("en-US");
        let listing = plain.project_with(&[("firefox.desktop", "Firefox")]);
        assert_eq!(
            listing["hint"].as_str(),
            Some(plain.localizer.message("launcher-search-hint").as_str()),
        );

        // Without rows there is nothing to choose and nothing to launch, so the
        // hint is the one binding that still does something.
        let mut no_match = fixture("en-US");
        no_match.query = "zzzz".to_owned();
        no_match.service = ServiceState::Ready {
            generation: 1,
            entries: 2,
            skipped: 0,
            directories: Vec::new(),
        };
        let mut loading = fixture("en-US");
        loading.service = ServiceState::Loading { generation: 2 };
        let mut unavailable = fixture("en-US");
        unavailable.service = ServiceState::Unavailable {
            generation: 1,
            failure: Failure::service(FailureKind::IndexUnavailable {
                detail: "no HOME".to_owned(),
            }),
        };
        let mut empty = Fixture::new("en-US", Vec::new());
        empty.service = ServiceState::Ready {
            generation: 1,
            entries: 0,
            skipped: 0,
            directories: Vec::new(),
        };

        for (name, state) in [
            ("no-match", &no_match),
            ("loading", &loading),
            ("unavailable", &unavailable),
            ("empty", &empty),
        ] {
            let value = state.project_with(&[]);
            assert!(
                value["tiles"].as_array().is_some_and(Vec::is_empty),
                "{name} still lists rows",
            );
            assert_eq!(
                value["hint"].as_str(),
                Some(state.localizer.message("launcher-hint-close").as_str()),
                "{name}: the hint promises keys that are not bound",
            );
            assert_ne!(
                value["hint"], listing["hint"],
                "{name}: an empty screen must not offer Enter",
            );
        }
    }

    #[test]
    fn the_hint_follows_the_negotiated_language_in_both_states() {
        let en = fixture("en-US");
        let zh = fixture("zh-CN");
        let en_hint = en.localizer.message("launcher-hint-close");
        let zh_hint = zh.localizer.message("launcher-hint-close");

        assert!(!en_hint.is_empty() && !zh_hint.is_empty());
        assert_ne!(en_hint, zh_hint, "the close hint is translated");
        assert_ne!(
            zh_hint,
            zh.localizer.message("launcher-search-hint"),
            "a close-only hint is not the list hint with fewer words",
        );

        let mut en_empty = fixture("en-US");
        en_empty.service = ServiceState::Loading { generation: 2 };
        let mut zh_empty = fixture("zh-CN");
        zh_empty.service = ServiceState::Loading { generation: 2 };
        assert_eq!(
            en_empty.project_with(&[])["hint"].as_str(),
            Some(en_hint.as_str())
        );
        assert_eq!(
            zh_empty.project_with(&[])["hint"].as_str(),
            Some(zh_hint.as_str())
        );
    }

    #[test]
    fn the_surface_title_stays_while_a_banner_replaces_the_grid() {
        let plain = fixture("en-US");
        let listing = plain.project_with(&[("firefox.desktop", "Firefox")]);
        assert_eq!(
            listing["heading"].as_str(),
            Some(plain.localizer.message("launcher-window-title").as_str()),
            "the top row carries the surface title",
        );

        let mut no_match = fixture("en-US");
        no_match.query = "zzzz".to_owned();
        no_match.service = ServiceState::Ready {
            generation: 1,
            entries: 2,
            skipped: 0,
            directories: Vec::new(),
        };
        let with_banner = no_match.project_with(&[]);
        assert_eq!(with_banner["banner"]["kind"].as_str(), Some("no-match"));
        assert_eq!(
            with_banner["heading"].as_str(),
            Some(no_match.localizer.message("launcher-window-title").as_str()),
            "the title belongs to the window, so a banner does not remove it",
        );
        assert!(
            with_banner["banner"]["title"]
                .as_str()
                .is_some_and(|title| !title.is_empty()),
            "a banner carries the message the grid would have shown",
        );
        assert_eq!(
            with_banner["title"].as_str(),
            Some(no_match.localizer.message("launcher-window-title").as_str()),
        );
        assert!(
            with_banner["tiles"].as_array().is_some_and(Vec::is_empty),
            "the banner is what the card holds instead of the tiles",
        );
    }

    #[test]
    fn the_projection_names_the_selected_row() {
        let mut selected = fixture("en-US");
        selected.selected = 1;
        let value = selected.project_with(&[
            ("firefox.desktop", "Firefox"),
            ("editor.desktop", "Text Editor"),
        ]);
        assert_eq!(value["selected"].as_u64(), Some(1));
        assert_eq!(value["tiles"][1]["selected"].as_bool(), Some(true));
        assert_eq!(value["tiles"][0]["selected"].as_bool(), Some(false));

        // A selection that is no row at all — the state before the first
        // result, and what a query that matched nothing leaves behind.
        let mut none = fixture("en-US");
        none.selected = usize::MAX;
        let unselected = none.project_with(&[("firefox.desktop", "Firefox")]);
        assert_eq!(
            unselected["selected"],
            Value::Null,
            "no row is the selection, so the index is null and not 0",
        );
        assert!(unselected["tiles"].as_array().is_some_and(|rows| {
            rows.iter()
                .all(|row| row["selected"].as_bool() != Some(true))
        }),);

        // And the same when there are no rows at all: a banner's screen.
        let mut banner = fixture("en-US");
        banner.service = ServiceState::Loading { generation: 2 };
        assert_eq!(banner.project_with(&[])["selected"], Value::Null);
    }

    #[test]
    fn the_action_height_clears_the_interaction_target_and_grows_with_the_type() {
        let plain = fixture("en-US");
        let value = plain.project_with(&[]);
        let metrics = Metrics::from_tokens(&plain.tokens);
        let height = value["theme"]["metrics"]["action_height"]
            .as_f64()
            .expect("a height");

        assert_eq!(height, px(theme::action_height(&plain.tokens)));
        assert!(
            height >= px(metrics.space_xl),
            "an action is never shorter than spacing.xl: {height} vs {}",
            px(metrics.space_xl),
        );

        // 200 % text: the label's own line box takes over and the control grows.
        let mut scaled = plain;
        scaled.tokens.typography.scale *= 2.0;
        let scaled_value = scaled.project_with(&[]);
        let scaled_height = scaled_value["theme"]["metrics"]["action_height"]
            .as_f64()
            .expect("a height");
        assert!(
            scaled_height > height,
            "the action height follows the font scale: {scaled_height} vs {height}",
        );
        assert_eq!(
            scaled_height,
            px(theme::action_height(&scaled.tokens)),
            "the projected height is the derivation, not a second formula",
        );
    }

    #[test]
    fn the_motion_durations_are_the_token_values() {
        let fixture = fixture("en-US");
        let value = fixture.project_with(&[]);
        let motion = &value["theme"]["motion"];
        let tokens = &fixture.tokens.motion;

        assert_eq!(
            motion["fast"].as_u64(),
            Some(u64::from(tokens.fast_ms)),
            "the fast duration is the token",
        );
        assert_eq!(motion["normal"].as_u64(), Some(u64::from(tokens.normal_ms)));
        assert_eq!(motion["slow"].as_u64(), Some(u64::from(tokens.slow_ms)));
        assert!(tokens.fast_ms <= tokens.normal_ms && tokens.normal_ms <= tokens.slow_ms);
        // The two durations this surface uses — a hover and a state change — sit
        // under the guidelines' §8 ceiling. `slow` is the entering/leaving surface
        // duration the token set defines and this surface does not use.
        assert!(
            tokens.fast_ms < 300 && tokens.normal_ms < 300,
            "guidelines §8: a UI transition stays under 300 ms",
        );
    }

    #[test]
    fn the_grid_is_the_placement_grid() {
        let plain = fixture("en-US");
        let value = plain.project_with(&[]);
        let grid = &plain.placement.grid;

        assert_eq!(
            value["grid"]["columns"].as_u64(),
            Some(grid.columns as u64),
            "the drawn columns are the keyboard's columns",
        );
        assert_eq!(
            value["grid"]["cell_width"].as_f64(),
            Some(px(grid.cell_width))
        );
        assert_eq!(
            value["grid"]["cell_height"].as_f64(),
            Some(px(grid.cell_height))
        );
        assert_eq!(value["grid"]["gap"].as_f64(), Some(px(grid.gap)));
        assert_eq!(value["grid"]["image"].as_f64(), Some(px(grid.image)));
        assert_eq!(
            value["grid"]["label_lines"].as_u64(),
            Some(crate::theme::TILE_LABEL_LINES as u64),
            "a tile's label wraps to the lines the view draws",
        );
        assert!(grid.columns >= 1);
    }

    #[test]
    fn the_chips_are_the_categories_with_entries_plus_all() {
        let mut development = AppIndex::test_entry("dev.desktop", "Development Tool", "dev");
        development.categories = vec!["Development".to_owned(), "IDE".to_owned()];
        let mut game = AppIndex::test_entry("game.desktop", "Game", "game");
        game.categories = vec!["Game".to_owned()];
        let mut bare = AppIndex::test_entry("bare.desktop", "Bare", "bare");
        bare.categories = Vec::new();

        let plain = Fixture::new("en-US", vec![development, game, bare]);
        let value = plain.project_with(&[]);
        let chips = value["chips"].as_array().expect("a chip row");
        let ids: Vec<&str> = chips
            .iter()
            .filter_map(|chip| chip["id"].as_str())
            .collect();

        assert_eq!(
            ids,
            ["all", "development", "game", "other"],
            "only the categories with entries, in the fixed order, plus all",
        );
        assert_eq!(
            value["category"].as_str(),
            Some("all"),
            "the default filter is the unfiltered browse",
        );
        assert_eq!(
            chips
                .iter()
                .filter(|chip| chip["selected"].as_bool() == Some(true))
                .count(),
            1,
            "exactly one chip is the filter",
        );
        assert_eq!(
            chips[1]["label"].as_str(),
            Some(
                plain
                    .localizer
                    .message("launcher-category-development")
                    .as_str()
            ),
            "a chip's label comes from the catalog",
        );
        // The selected chip is the accent fill, like a selected row.
        assert_eq!(
            chips[0]["style"]["background"].as_str(),
            Some(theme::qml_color(plain.tokens.colors.accent).as_str()),
        );
        assert_eq!(
            chips[0]["style"]["text"].as_str(),
            Some(theme::qml_color(plain.tokens.colors.on_accent).as_str()),
        );
        assert_ne!(
            chips[1]["style"]["background"], chips[0]["style"]["background"],
            "an unselected chip has no fill",
        );
    }

    #[test]
    fn the_chip_labels_follow_the_negotiated_language() {
        let mut game = AppIndex::test_entry("game.desktop", "Game", "game");
        game.categories = vec!["Game".to_owned()];

        let en = Fixture::new("en-US", vec![game.clone()]).project_with(&[]);
        let zh = Fixture::new("zh-CN", vec![game]).project_with(&[]);

        let en_label = en["chips"][1]["label"].as_str().expect("a label");
        let zh_label = zh["chips"][1]["label"].as_str().expect("a label");
        assert!(!en_label.is_empty() && !zh_label.is_empty());
        assert_ne!(en_label, zh_label, "the chips are translated");
    }

    #[test]
    fn a_tile_carries_its_icon_file_as_a_file_url() {
        let mut with_icon = AppIndex::test_entry("icon.desktop", "Icon", "icon");
        with_icon.icon = Some("icon".to_owned());
        with_icon.icon_path = Some(std::path::PathBuf::from(
            "/usr/share/icons/hicolor/96x96/apps/icon.png",
        ));
        let with_space = {
            let mut entry = AppIndex::test_entry("spaced.desktop", "Spaced", "spaced");
            entry.icon = Some("spaced".to_owned());
            entry.icon_path = Some(std::path::PathBuf::from("/tmp/My Icons/spaced.png"));
            entry
        };
        let bare = AppIndex::test_entry("bare.desktop", "Bare", "bare");

        let plain = Fixture::new("en-US", vec![with_icon, with_space, bare]);
        let value = plain.project_with(&[
            ("icon.desktop", "Icon"),
            ("spaced.desktop", "Spaced"),
            ("bare.desktop", "Bare"),
        ]);

        assert_eq!(
            value["tiles"][0]["icon_url"].as_str(),
            Some("file:///usr/share/icons/hicolor/96x96/apps/icon.png"),
        );
        assert_eq!(
            value["tiles"][1]["icon_url"].as_str(),
            Some("file:///tmp/My%20Icons/spaced.png"),
            "a space in the path is percent-encoded for QML",
        );
        assert_eq!(
            value["tiles"][2]["icon_url"],
            Value::Null,
            "an entry with no icon file falls back to its letter",
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
            "notice",
            "selected",
            "category",
            "tiles",
            "chips",
            "grid",
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
        for key in ["name", "colors", "font", "metrics", "styles", "motion"] {
            assert!(theme.get(key).is_some(), "the theme lacks {key}");
        }
        for style in ["window", "card", "field", "action"] {
            assert!(
                theme["styles"].get(style).is_some(),
                "the styles lack {style}"
            );
        }
        // The fields the list and the actions bind to: how tall an action is,
        // and which durations the motion may use.
        assert!(
            theme["metrics"].get("action_height").is_some(),
            "the metrics lack action_height"
        );
        for key in ["fast", "normal", "slow"] {
            assert!(theme["motion"].get(key).is_some(), "the motion lacks {key}");
        }
    }

    #[test]
    fn the_fallback_projection_is_total_and_empty() {
        let value = empty_projection();

        assert_eq!(value["tiles"].as_array().map(Vec::len), Some(0));
        assert_eq!(value["banner"], Value::Null);
        assert_eq!(value["status"], Value::Null);
        assert_eq!(value["title"].as_str(), Some(""));
        assert_eq!(value["heading"].as_str(), Some(""));
        assert_eq!(value["selected"], Value::Null);
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
