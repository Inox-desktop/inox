// A glass button: a pane with a centred label and a focus ring.
//
// A stateless renderer. `pressed`, `focused` and `enabled` are states the caller
// decided in Rust and projected; this file turns them into pixels and nothing
// else — it holds no input handler, so a click never reaches Qt (the panel's
// `layer.rs` owns hit testing, ordering and dispatch). Every colour, radius and
// font value is a property, filled from the design tokens by the caller.
import QtQuick

Item {
    id: button

    // The label. Caller-supplied (Fluent), never a literal in this file.
    property string text: ""
    // Font family and size from the `typography` tokens.
    property string fontFamily: ""
    property real fontSize: 0
    // Corner radius from the `radii` tokens.
    property real radius: 0
    // Hairline thickness for the pane's two edges.
    property real hairline: 1

    // Glass tokens: the tile, its emphasised (pressed) variant, both hairlines.
    property color tint: "transparent"
    property color tintPressed: "transparent"
    property color highlight: "transparent"
    property color shade: "transparent"
    // The label colour.
    property color textColor: "transparent"
    // The keyboard focus ring's colour and thickness (a token, not an invention).
    property color focusRing: "transparent"
    property real focusRingWidth: 0

    // State, decided by the caller. `enabled` is the inherited `Item.enabled`:
    // the component exports it to the accessibility tree and draws no state of its
    // own for it — the caller's chosen tint is the disabled look.
    property bool pressed: false
    property bool focused: false

    Accessible.role: Accessible.Button
    Accessible.name: button.text
    Accessible.pressed: button.pressed
    Accessible.focused: button.focused

    GlassPane {
        anchors.fill: parent
        tint: button.pressed ? button.tintPressed : button.tint
        highlight: button.highlight
        shade: button.shade
        radius: button.radius
        hairline: button.hairline
    }

    Text {
        anchors.fill: parent
        anchors.leftMargin: button.radius
        anchors.rightMargin: button.radius
        text: button.text
        color: button.textColor
        font.family: button.fontFamily
        font.pixelSize: button.fontSize
        horizontalAlignment: Text.AlignHCenter
        verticalAlignment: Text.AlignVCenter
        elide: Text.ElideRight
    }

    Rectangle {
        // The ring is drawn inside the button's own bounds: glass must not change
        // the hit area or the layout of its neighbours (§12).
        anchors.fill: parent
        anchors.margins: button.focusRingWidth / 2
        visible: button.focused && button.focusRingWidth > 0
        color: "transparent"
        border.width: button.focusRingWidth
        border.color: button.focusRing
        radius: Math.max(0, button.radius - button.focusRingWidth / 2)
        antialiasing: true
    }
}
