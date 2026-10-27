// The glass pane: what a component draws of a liquid-glass material.
//
// A pane is a translucent tint laid over the compositor's softened and refracted
// backdrop, with a hairline highlight along its top edge and a shade along its
// bottom one (`docs/quality/ui-guidelines.md` §12, ADR 0040). Everything visible
// here is a property: the tint, both hairlines, the radius and the hairline width
// arrive from the design tokens through the caller's projection
// (`inox_glass::catalog_model`, `crates/inox-panel/src/theme.rs`).
//
// Nothing in this file picks a colour, a size or a sentence, and nothing here
// handles input: the state (pressed, focused, enabled, …) is drawn by the caller's
// chosen tint, and hit testing stays with the Rust projection that owns it. A
// default of `transparent` / `0` is the structural "nothing to draw yet" value,
// which is what a pane looks like before a projection arrives.
import QtQuick

Rectangle {
    id: pane

    // The tile of the material: a token colour, translucent by design.
    property color tint: "transparent"
    // Hairline along the top edge — what reads as "glass".
    property color highlight: "transparent"
    // Darkening along the bottom edge.
    property color shade: "transparent"
    // The corner radius is the inherited `Rectangle.radius`: a `radii` token the
    // caller sets, defaulting to square corners.
    // Thickness of the two hairlines: a structural AA value, from `spacing.xs`.
    property real hairline: 1
    // Whether the caller wants the two edge hairlines drawn at all.
    property bool highlighted: true
    property bool shaded: true

    color: pane.tint
    antialiasing: true

    // The hairlines start where the straight edge does, so a rounded corner is
    // never given a square highlight over its curve.
    Rectangle {
        id: highlightEdge

        visible: pane.highlighted && pane.hairline > 0
        x: pane.radius
        y: 0
        width: Math.max(0, pane.width - 2 * pane.radius)
        height: pane.hairline
        color: pane.highlight
    }

    Rectangle {
        id: shadeEdge

        visible: pane.shaded && pane.hairline > 0
        x: pane.radius
        y: pane.height - pane.hairline
        width: Math.max(0, pane.width - 2 * pane.radius)
        height: pane.hairline
        color: pane.shade
    }
}
