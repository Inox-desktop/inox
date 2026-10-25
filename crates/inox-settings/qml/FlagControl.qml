// A switch and the word it is showing.
//
// A switch is a control that is on or off rather than one of a set, so "on" is
// drawn as the selection: the accent background and its own text colour.
import QtQuick
import com.inox.settings.qml

ControlButton {
    id: flag

    // The field being edited and the element the action is sent to.
    required property var field
    required property var bridge

    label: flag.field.control.label
    selected: flag.field.control.on
    focused: flag.field.focused
    enabled: flag.field.enabled
    focusId: flag.field.focusId
    onActivated: flag.bridge.dispatch(JSON.stringify({
        type: "field-set",
        field: flag.field.field,
        value: { kind: "flag", on: !flag.field.control.on }
    }))
}
