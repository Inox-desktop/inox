//! The catalog page's projection: design tokens in, one JSON string out.
//!
//! `qml/CatalogWindow.qml` draws all five glass components from a single
//! `modelJson` property, exactly like every other L3 QML renderer
//! (`crates/inox-panel/src/view.rs`): the QML holds no colour, no sentence and no
//! token of its own, and a host's offscreen test can render the page, grab it and
//! assert pixels.
//!
//! The projection is produced here rather than hand-written in QML because the
//! alternative is a catalog page with literal colours in it, which is precisely
//! what root AGENTS.md §4 forbids. A consumer renders the catalog with:
//!
//! ```no_run
//! use inox_config::Theme;
//!
//! let model = inox_glass::catalog_model(&Theme::builtin_dark());
//! // `host.set_model(&model)` — the panel's offscreen host reads `modelJson`.
//! assert!(!model.is_empty());
//! ```
//!
//! # The model
//!
//! Every rectangle carries its own `x`, `y`, `width` and `height` in logical
//! pixels, so a test reads the geometry out of the model instead of depending on
//! a layout it cannot see:
//!
//! ```text
//! { "width", "height",               page size (advisory: a host may resize)
//!   "background",                    the page fill — the structural
//!                                    "transparent" sentinel, not a design colour
//!   "fontFamily", "fontSize", "hairline",
//!   "pane":   { x, y, width, height, radius, tint, highlight, shade },
//!   "button": { x, y, width, height, radius, text, tint, tintPressed, highlight,
//!               shade, textColor, focusRing, focusRingWidth,
//!               pressed, focused, enabled },
//!   "toggle": { x, y, width, height, radius, label, trackTint, trackTintChecked,
//!               knobColor, checked, enabled },
//!   "slider": { x, y, width, height, trackRadius, label, trackTint, fillTint,
//!               knobColor, knobSize, value, enabled },
//!   "tabBar": { x, y, width, height, radius, accessibleName, labels,
//!               currentIndex, tint, selectedTint, highlight, shade, textColor,
//!               selectedTextColor, enabled } }
//! ```
//!
//! `text`, `label`, `accessibleName` and `labels` are **empty** in this
//! projection on purpose: user-visible strings come from Fluent through the
//! component's caller, never from the crate that draws the glass. A consumer that
//! wants labelled components replaces those fields with translated messages.
//!
//! Every colour is a token (`tokens.glass`, `tokens.colors`) and every size is a
//! token (`tokens.spacing`, `tokens.radii`, `tokens.typography`). The only
//! numbers written down here are structural: the page is a test page, so its
//! width is ten control heights, the button is four control heights wide, and the
//! toggle track is two short-side... — see the constants below.

use inox_config::{Rgba, Theme};
use serde_json::{Value, json};

/// How many control heights wide the catalog page is (a structural page size).
const PAGE_WIDTH_IN_CONTROLS: u32 = 10;

/// How many control heights wide the sample button is.
const BUTTON_WIDTH_IN_CONTROLS: u32 = 4;

/// How many band thicknesses wide the toggle track is.
const TOGGLE_WIDTH_IN_BANDS: u32 = 2;

/// How many tabs the sample tab bar shows.
const TAB_COUNT: usize = 3;

/// How full the sample slider is.
const SLIDER_VALUE: f64 = 0.5;

/// The structural "no colour" value: nothing is painted here.
///
/// It is not a design colour and no theme can set it; it matches the sentinel
/// `qml/CatalogWindow.qml` starts from before a projection arrives, and the
/// panel's `theme::TRANSPARENT`.
const TRANSPARENT: &str = "#00000000";

/// The catalog page's projection for one theme.
pub fn from_theme(tokens: &Theme) -> String {
    layout(tokens).to_string()
}

/// The projection, as a JSON value (the page size included).
fn layout(tokens: &Theme) -> Value {
    let page_width = u32::from(tokens.spacing.xl) * PAGE_WIDTH_IN_CONTROLS;
    let pad = u32::from(tokens.spacing.lg);
    let gap = u32::from(tokens.spacing.md);
    let control_height = u32::from(tokens.spacing.xl);
    let band = u32::from(tokens.spacing.lg);
    let inner_width = page_width - 2 * pad;

    let glass = &tokens.glass;
    let colours = &tokens.colors;
    let colour = |value: Rgba| text(value);

    let mut cursor = pad;

    let pane = json!({
        "x": pad,
        "y": cursor,
        "width": inner_width,
        "height": control_height * 2,
        "radius": tokens.radii.lg,
        "tint": colour(glass.tint),
        "highlight": colour(glass.highlight),
        "shade": colour(glass.shade),
    });
    cursor += control_height * 2 + gap;

    let row = cursor;
    let button_width = control_height * BUTTON_WIDTH_IN_CONTROLS;
    let button = json!({
        "x": pad,
        "y": row,
        "width": button_width,
        "height": control_height,
        "radius": tokens.radii.sm,
        "text": "",
        "tint": colour(glass.tint),
        "tintPressed": colour(glass.tint_strong),
        "highlight": colour(glass.highlight),
        "shade": colour(glass.shade),
        "textColor": colour(colours.text),
        "focusRing": colour(colours.accent),
        "focusRingWidth": f32::from(tokens.spacing.xs),
        "pressed": false,
        "focused": false,
        "enabled": true,
    });

    let toggle_width = band * TOGGLE_WIDTH_IN_BANDS;
    let toggle = json!({
        "x": pad + button_width + gap,
        "y": row + (control_height - band) / 2,
        "width": toggle_width,
        "height": band,
        "radius": band / 2,
        "label": "",
        "trackTint": colour(colours.surface_variant),
        "trackTintChecked": colour(colours.accent),
        "knobColor": colour(colours.on_accent),
        "checked": true,
        "enabled": true,
    });
    cursor += control_height + gap;

    let slider = json!({
        "x": pad,
        "y": cursor,
        "width": inner_width,
        "height": band,
        "trackRadius": band / 2,
        "label": "",
        "trackTint": colour(colours.surface_variant),
        "fillTint": colour(colours.accent),
        "knobColor": colour(colours.on_accent),
        "knobSize": band,
        "value": SLIDER_VALUE,
        "enabled": true,
    });
    cursor += band + gap;

    let tab_bar = json!({
        "x": pad,
        "y": cursor,
        "width": inner_width,
        "height": control_height,
        "radius": tokens.radii.sm,
        "accessibleName": "",
        "labels": vec![""; TAB_COUNT],
        "currentIndex": 0,
        "tint": colour(glass.tint),
        "selectedTint": colour(glass.tint_strong),
        "highlight": colour(glass.highlight),
        "shade": colour(glass.shade),
        "textColor": colour(colours.text),
        "selectedTextColor": colour(colours.text),
        "enabled": true,
    });
    cursor += control_height + pad;

    json!({
        "width": page_width,
        "height": cursor,
        "background": TRANSPARENT,
        "fontFamily": tokens.typography.font_family,
        "fontSize": f32::from(tokens.spacing.xl) / 2.0 * tokens.typography.scale,
        "hairline": f32::from(tokens.spacing.xs) / 2.0,
        "pane": pane,
        "button": button,
        "toggle": toggle,
        "slider": slider,
        "tabBar": tab_bar,
    })
}

/// A token colour, as the QML renderer wants it: Qt's `#AARRGGBB`.
///
/// Qt parses `#AARRGGBB` as ARGB with the alpha first, which is the value the
/// token holds; `#RRGGBBAA` would make Qt read the alpha as a blue channel.
fn text(value: Rgba) -> String {
    format!(
        "#{:02X}{:02X}{:02X}{:02X}",
        value.a, value.r, value.g, value.b
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use inox_config::Appearance;

    fn dark() -> Value {
        let model = from_theme(&Theme::builtin_dark());

        serde_json::from_str(&model).expect("the projection is JSON")
    }

    #[test]
    fn the_projection_draws_all_five_components_plus_the_page() {
        let model = dark();

        for key in ["pane", "button", "toggle", "slider", "tabBar"] {
            assert!(
                model.get(key).is_some_and(Value::is_object),
                "the catalog draws every component, {key} is missing: {model}"
            );
        }

        assert_eq!(
            model.get("background").and_then(Value::as_str),
            Some(TRANSPARENT),
            "the page's own fill is the structural no-colour sentinel"
        );
    }

    #[test]
    fn every_rectangle_carries_its_own_geometry() {
        let model = dark();

        for key in ["pane", "button", "toggle", "slider", "tabBar"] {
            let component = &model[key];

            for field in ["x", "y", "width", "height"] {
                assert!(
                    component.get(field).and_then(Value::as_u64).is_some(),
                    "{key}.{field} must be a number a test can read: {component}"
                );
            }

            let width = component["width"].as_u64().expect("width");
            let height = component["height"].as_u64().expect("height");
            assert!(width > 0 && height > 0, "{key} is drawn at a size");
        }

        let page_width = model["width"].as_u64().expect("page width");
        let page_height = model["height"].as_u64().expect("page height");
        let pane_bottom = model["pane"]["y"].as_u64().expect("pane y")
            + model["pane"]["height"].as_u64().expect("pane height");
        let tab_bottom = model["tabBar"]["y"].as_u64().expect("tab y")
            + model["tabBar"]["height"].as_u64().expect("tab height");

        assert!(
            tab_bottom < page_height && pane_bottom < page_height,
            "every component is inside the page: {model}"
        );
        assert!(
            model["pane"]["x"].as_u64().expect("pane x")
                + model["pane"]["width"].as_u64().expect("pane width")
                <= page_width,
            "the pane is inside the page: {model}"
        );
    }

    #[test]
    fn the_colours_are_the_theme_tokens() {
        let tokens = Theme::builtin_dark();
        let model = dark();

        assert_eq!(
            model["pane"]["tint"].as_str(),
            Some(text(tokens.glass.tint).as_str())
        );
        assert_eq!(
            model["pane"]["highlight"].as_str(),
            Some(text(tokens.glass.highlight).as_str())
        );
        assert_eq!(
            model["toggle"]["trackTintChecked"].as_str(),
            Some(text(tokens.colors.accent).as_str()),
            "a checked toggle is the accent token, not a colour of this crate's own"
        );
        assert_eq!(
            model["slider"]["fillTint"].as_str(),
            Some(text(tokens.colors.accent).as_str())
        );
        assert_eq!(
            model["pane"]["radius"].as_u64(),
            Some(u64::from(tokens.radii.lg))
        );
    }

    #[test]
    fn the_light_and_dark_themes_project_different_glass() {
        let dark = from_theme(&Theme::builtin_dark());
        let light = from_theme(&Theme::builtin_light());

        assert_ne!(dark, light, "a theme change repaints the catalog");
        assert_eq!(Theme::builtin_dark().appearance, Appearance::Dark);
        assert_eq!(Theme::builtin_light().appearance, Appearance::Light);
    }

    #[test]
    fn no_user_visible_string_is_written_down_here() {
        let model = dark();

        assert_eq!(model["button"]["text"].as_str(), Some(""));
        assert_eq!(model["toggle"]["label"].as_str(), Some(""));
        assert_eq!(model["slider"]["label"].as_str(), Some(""));
        assert_eq!(model["tabBar"]["accessibleName"].as_str(), Some(""));
        assert_eq!(
            model["tabBar"]["labels"]
                .as_array()
                .expect("labels is a list")
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>(),
            vec![""; TAB_COUNT],
            "labels come from Fluent through the caller, never from this crate"
        );
    }

    #[test]
    fn the_sample_states_are_the_ones_a_pixel_test_asserts() {
        let model = dark();

        assert_eq!(
            model["toggle"]["checked"].as_bool(),
            Some(true),
            "the catalog shows the checked colour"
        );
        assert_eq!(model["slider"]["value"].as_f64(), Some(SLIDER_VALUE));
        assert_eq!(model["tabBar"]["currentIndex"].as_u64(), Some(0));
        assert!(
            model["button"]["enabled"].as_bool() == Some(true),
            "the sample button is drawn in its normal state"
        );
    }

    #[test]
    fn a_theme_with_different_spacing_projects_a_different_page() {
        let mut tokens = Theme::builtin_dark();

        tokens.spacing.xl = 48;

        let scaled = layout(&tokens);
        assert_eq!(
            scaled["width"].as_u64(),
            Some(u64::from(PAGE_WIDTH_IN_CONTROLS) * 48),
            "the page is sized from the spacing token, not from a literal"
        );
    }

    #[test]
    fn every_colour_is_a_qt_argb_literal() {
        let model = dark();

        let mut checked = 0;
        collect_colours(&model, &mut checked);

        assert!(checked > 0, "the projection does carry colours");
    }

    fn collect_colours(value: &Value, checked: &mut usize) {
        match value {
            Value::String(colour) if colour.starts_with('#') && colour.len() == 9 => {
                assert!(
                    colour[1..].chars().all(|digit| digit.is_ascii_hexdigit()),
                    "{colour} is not hexadecimal"
                );
                *checked += 1;
            }
            Value::Object(fields) => {
                for field in fields.values() {
                    collect_colours(field, checked);
                }
            }
            Value::Array(items) => {
                for item in items {
                    collect_colours(item, checked);
                }
            }
            _ => {}
        }
    }
}
