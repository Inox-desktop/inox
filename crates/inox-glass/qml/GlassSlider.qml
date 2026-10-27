// A glass slider: a filled track with a knob on the fill's edge.
//
// The component draws the value it is handed and never changes it: dragging,
// keyboard steps and the value's range all live in the Rust projection, which is
// what keeps the glass layer a pure renderer (§12, and the panel's contract). The
// fill ends exactly `value` along the track, which is also what an offscreen pixel
// test asserts.
//
// `value` is the position along the track in the range `0..1`; a caller with a
// different range divides it before projecting. Out-of-range values are drawn at
// the nearest end instead of spilling outside the track.
import QtQuick

Item {
    id: slider

    // Position along the track, `0..1`, decided by the caller.
    property real value: 0
    // `enabled` is the inherited `Item.enabled`: the caller projects the state,
    // this component only exports it to the accessibility tree.
    // The accessible name (Fluent, from the caller). Not drawn here.
    property string label: ""

    // Tokens: the empty track, the filled part, the knob.
    property color trackTint: "transparent"
    property color fillTint: "transparent"
    property color knobColor: "transparent"
    // Corner radius of the track (a `radii` token), and the knob's diameter.
    property real trackRadius: 0
    property real knobSize: 0

    readonly property real position: Math.max(0, Math.min(1, slider.value))

    Accessible.role: Accessible.Slider
    Accessible.name: slider.label

    Rectangle {
        id: track

        anchors.fill: parent
        color: slider.trackTint
        radius: slider.trackRadius
        antialiasing: true
    }

    Rectangle {
        id: fill

        anchors.left: track.left
        anchors.verticalCenter: track.verticalCenter
        width: track.width * slider.position
        height: track.height
        color: slider.fillTint
        radius: slider.trackRadius
        antialiasing: true
    }

    Rectangle {
        id: knob

        // Centred on the fill's edge, and kept inside the track at both ends so a
        // control at `0` or `1` never paints over its neighbour.
        x: Math.max(0, Math.min(track.width - width, fill.width - width / 2))
        anchors.verticalCenter: track.verticalCenter
        width: slider.knobSize
        height: slider.knobSize
        radius: width / 2
        color: slider.knobColor
        antialiasing: true
    }
}
