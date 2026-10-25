// A number between two limits, its unit, and the two buttons that walk it.
//
// The number, its unit and its steps are one control: they never wrap away from
// each other, whatever the font scale (Q02). The step buttons stop at the ends of
// the range they are drawn at, and the keyboard can still wrap past the end —
// which is what keeps every value reachable from the keyboard alone (Q01).
import QtQuick
import com.inox.settings.qml

Row {
    id: number

    required property var theme
    required property var bridge
    required property var field
    property bool focused: false

    readonly property var geo: theme.geometry
    readonly property var colors: theme.colors
    readonly property var value: field.control

    spacing: geo.inlineGap

    ControlButton {
        theme: number.theme
        label: number.value.decrease
        enabled: number.field.enabled && !number.value.atMin
        onActivated: number.bridge.dispatch(JSON.stringify({
            type: "field-stepped",
            field: number.field.field,
            delta: -1
        }))
    }

    TextFieldControl {
        width: number.geo.shortWidth
        theme: number.theme
        bridge: number.bridge
        field: number.field
        focused: number.focused
    }

    Text {
        anchors.verticalCenter: parent.verticalCenter
        text: number.value.unit
        color: number.colors.textMuted
        font.family: number.theme.fontFamily
        font.pixelSize: number.geo.captionSize

        Accessible.role: Accessible.StaticText
        Accessible.name: number.value.unit
    }

    ControlButton {
        theme: number.theme
        label: number.value.increase
        enabled: number.field.enabled && !number.value.atMax
        onActivated: number.bridge.dispatch(JSON.stringify({
            type: "field-stepped",
            field: number.field.field,
            delta: 1
        }))
    }
}
