// A segmented control: a row of options, exactly one of which holds the current
// value.
//
// An option a user did not choose can still be shown as the value — a hand-written
// language tag, a custom wallpaper path — because the projection carries the
// file's own value beside the options it offers. Choosing an option is a value
// change, not a text edit.
import QtQuick
import com.inox.settings.qml

Flow {
    id: choices

    required property var theme
    required property var bridge
    required property var field

    readonly property var geo: theme.geometry
    readonly property var options: field.control.options

    spacing: geo.inlineGap

    Repeater {
        model: choices.options

        delegate: ControlButton {
            required property var modelData
            theme: choices.theme
            label: modelData.label
            selected: modelData.selected
            focused: modelData.focused
            enabled: choices.field.enabled
            focusId: modelData.focusId
            onActivated: choices.bridge.dispatch(JSON.stringify({
                type: "field-set",
                field: choices.field.field,
                value: { kind: "choice", id: modelData.id }
            }))
        }
    }
}
