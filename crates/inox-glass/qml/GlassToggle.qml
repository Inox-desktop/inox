// A glass toggle: the switch itself, without a label.
//
// The component is the track, so its width and height are the caller's geometry
// and nothing is arranged behind the caller's back. The visible label is drawn by
// the caller next to it (like every other control in an L3 view, the text comes
// from a Fluent message, not from the widget); `label` here is the accessible
// name only.
//
// A stateless renderer: `checked` is a projection of Rust state, and there is no
// input handler — clicking is hit-tested by the component's owner. The knob does
// not animate: `docs/quality/ui-guidelines.md` §12 forbids motion in the glass
// layer, and an offscreen pixel test benefits from a frame it can predict.
import QtQuick

Rectangle {
    id: toggle

    // State, decided by the caller. `enabled` is the inherited `Item.enabled`:
    // Qt Quick exposes that as the accessibility `enabled` state, and the caller's
    // chosen tint is the disabled look.
    property bool checked: false

    // The accessible name (Fluent, from the caller). Not drawn here.
    property string label: ""

    // Tokens: the track in both states and the knob colour.
    property color trackTint: "transparent"
    property color trackTintChecked: "transparent"
    property color knobColor: "transparent"

    color: toggle.checked ? toggle.trackTintChecked : toggle.trackTint
    radius: toggle.radius
    antialiasing: true

    Accessible.role: Accessible.CheckBox
    Accessible.name: toggle.label
    Accessible.checkable: true
    Accessible.checked: toggle.checked

    Rectangle {
        id: knob

        // The knob is a circle as wide as the track is tall, and it sits at one
        // end of the track: checked travels to the far end.
        width: toggle.height
        height: toggle.height
        x: toggle.checked ? toggle.width - width : 0
        y: 0
        radius: width / 2
        color: toggle.knobColor
        antialiasing: true
    }
}
