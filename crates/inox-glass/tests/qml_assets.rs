//! The asset rules, asserted instead of promised.
//!
//! The QML files in `qml/` are shipped to consumers as *assets*, so `cargo test`
//! is the only place their rules can be checked without a Qt harness: a stateless
//! renderer (no input handlers), token-driven (no colours, sizes or sentences of
//! its own), accessible (a role and a name on every interactive component), and a
//! catalog page that draws all five components from one projection.
//!
//! This test is deliberately textual. It cannot prove the files *look* right —
//! that is the panel's offscreen pixel test (`CatalogWindow.qml` is the designed
//! reference) — but it does prove that a later edit cannot quietly reintroduce the
//! things ADR 0040 and `docs/quality/ui-guidelines.md` §12 rule out.

use std::path::Path;

use inox_glass::{QML_FILES, catalog_qml, qml_files};

/// The components that take user input, so they must expose an accessible role
/// and name.
const INTERACTIVE: [(&str, &str, &str); 4] = [
    ("GlassButton.qml", "Accessible.Button", "text"),
    ("GlassToggle.qml", "Accessible.CheckBox", "label"),
    ("GlassSlider.qml", "Accessible.Slider", "label"),
    (
        "GlassTabBar.qml",
        "Accessible.PageTabList",
        "accessibleName",
    ),
];

/// The structural "no colour" sentinel: the only literal allowed in an asset.
///
/// It is what a transparent page is filled with before a projection arrives and
/// what `inox_glass::catalog_model` writes for it; it is not a design colour and
/// no theme can produce it.
const STRUCTURAL_TRANSPARENT: &str = "#00000000";

fn read(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("qml").join(name);

    std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("cannot read {path:?}: {error}"))
}

/// The text of a QML file with its comment lines removed.
///
/// The rules below are about what the renderer *does*; prose that explains a rule
/// must not trip the check for it (the assets are heavily commented on purpose).
fn code(name: &str) -> String {
    read(name)
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn no_asset_handles_input() {
    // Glass is a visual layer: hit testing, focus order and dispatch stay in the
    // Rust projection that owns them (ADR 0040 §3, ui-guidelines §12).
    let forbidden = [
        "MouseArea",
        "TapHandler",
        "DragHandler",
        "HoverHandler",
        "WheelHandler",
        "Keys.",
        "onClicked",
        "onPressed",
        "onReleased",
        "onToggled",
        "onValueChanged",
    ];

    for name in QML_FILES {
        let text = code(name);

        for token in forbidden {
            assert!(
                !text.contains(token),
                "{name} must not handle input, but it contains `{token}`"
            );
        }
    }
}

#[test]
fn no_asset_hardcodes_a_colour() {
    for name in QML_FILES {
        let text = code(name);

        for (index, line) in text.lines().enumerate() {
            let mut rest = line;

            while let Some(hash) = rest.find('#') {
                let digits: String = rest[hash + 1..]
                    .chars()
                    .take_while(|character| character.is_ascii_alphanumeric())
                    .collect();
                let literal = format!("#{digits}");
                rest = &rest[hash + 1 + digits.len()..];

                assert!(
                    literal == STRUCTURAL_TRANSPARENT,
                    "{name}:{}: `{literal}` is a colour literal; an asset may only use \
                     `transparent` or the structural `{STRUCTURAL_TRANSPARENT}` sentinel, \
                     every design colour arrives as a property",
                    index + 1
                );
            }
        }
    }
}

#[test]
fn no_asset_hardcodes_a_user_visible_sentence() {
    // Fluent owns every string (root AGENTS.md §4). A label, a text or an
    // accessible name is a property the caller fills in.
    for name in QML_FILES {
        for (index, line) in code(name).lines().enumerate() {
            let trimmed = line.trim();

            // A property *declaration* states the type and the empty default; it
            // is not a literal sentence.
            if trimmed.starts_with("property ") || trimmed.starts_with("required property ") {
                continue;
            }

            for property in ["text:", "label:", "accessibleName:", "name:"] {
                let Some(position) = trimmed.find(property) else {
                    continue;
                };

                let value = trimmed[position + property.len()..].trim_start();

                assert!(
                    !value.starts_with('"'),
                    "{name}:{}: `{trimmed}` writes a literal string; strings arrive as \
                     properties",
                    index + 1
                );
            }
        }
    }
}

#[test]
fn every_interactive_component_exposes_a_role_and_a_name() {
    for (name, role, name_property) in INTERACTIVE {
        let text = code(name);

        assert!(
            text.contains(&format!("Accessible.role: {role}")),
            "{name} must declare `{role}`"
        );
        assert!(
            text.contains("Accessible.name:"),
            "{name} must expose an accessible name"
        );
        assert!(
            text.contains(&format!("property string {name_property}:")),
            "{name} takes its accessible name as the `{name_property}` property"
        );
    }
}

#[test]
fn the_catalog_page_is_a_window_driven_by_the_projection() {
    let text = code("CatalogWindow.qml");

    assert!(
        text.contains("import QtQuick.Window"),
        "the catalog's root is a Window so a host can grab it"
    );
    assert!(text.contains("\nWindow {"), "the root object is a Window");
    assert!(
        text.contains("property string modelJson:"),
        "the host hands the page one JSON projection, like the panel's Main.qml"
    );
    assert!(
        text.contains("JSON.parse(root.modelJson)"),
        "the projection is parsed, not interpreted"
    );
    assert!(
        text.contains("property color hostBackdrop:"),
        "the host swaps the clear colour to reconstruct the alpha channel"
    );
    assert!(
        text.contains("color: root.hostBackdrop"),
        "the clear colour must be the host's, not the page's"
    );

    for component in [
        "GlassPane",
        "GlassButton",
        "GlassToggle",
        "GlassSlider",
        "GlassTabBar",
    ] {
        assert!(
            text.contains(&format!("{component} {{")),
            "the catalog draws {component}"
        );
    }
}

#[test]
fn the_catalog_page_is_reachable_from_the_asset_list() {
    assert!(
        qml_files().contains(&catalog_qml()),
        "a consumer that compiles `qml_files()` gets the catalog page too"
    );
    assert_eq!(
        QML_FILES
            .iter()
            .filter(|name| name.ends_with(".qml"))
            .count(),
        QML_FILES.len(),
        "every listed asset is a .qml file"
    );
}

#[test]
fn the_components_import_only_qt_quick() {
    // No QtQuick.Controls: the catalog is the only styling authority, and a
    // control style would put a second one in the binary. No Qt Wayland client
    // either (ADR 0040: the surface is the panel's, not Qt's).
    for name in QML_FILES {
        let text = code(name);

        for forbidden in ["QtQuick.Controls", "QtWayland", "Qt.labs"] {
            assert!(
                !text.contains(forbidden),
                "{name} must not import {forbidden}"
            );
        }
    }
}
