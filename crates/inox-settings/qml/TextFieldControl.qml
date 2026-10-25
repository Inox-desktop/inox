// An editable text value: a field's own text, a number's editable text, or one
// cell of a structured list.
//
// The value is the Rust projection's, kept exactly as the user typed it — an
// incomplete or overflowing number stays visible and blocks the save. Every
// keystroke is an action; the tree never holds a value of its own, so what the
// window shows and what it would write cannot drift apart.
import QtQuick

Item {
    id: control

    // The projection's theme.
    required property var theme
    // The Rust-backed element the actions are sent to.
    required property var bridge
    // The field being edited.
    required property var field
    // Whether the keyboard is on this field.
    property bool focused: false
    // The row and column, when this edits one cell of a list; -1 otherwise.
    property int row: -1
    property int column: -1

    readonly property var geo: theme.geometry
    readonly property var colors: theme.colors
    readonly property var input: theme.input
    readonly property var value: field.control
    readonly property bool numeric: value.kind === "number"
    readonly property bool refused: field.error !== null
    readonly property string shown: numeric ? value.input : value.value

    implicitWidth: 0
    implicitHeight: geo.controlHeight

    Rectangle {
        anchors.fill: parent
        radius: control.geo.radius
        color: control.field.enabled ? control.input.background : control.input.backgroundDisabled
        border.width: control.focused || control.refused
                      ? control.input.focusWidth
                      : control.input.borderWidth
        border.color: control.borderColour()
    }

    HoverHandler {
        id: hover
    }

    TextInput {
        id: text

        anchors.fill: parent
        anchors.leftMargin: control.geo.controlPaddingHorizontal
        anchors.rightMargin: control.geo.controlPaddingHorizontal
        verticalAlignment: TextInput.AlignVCenter
        color: control.field.enabled ? control.input.value : control.input.valueDisabled
        selectionColor: control.input.selection
        selectedTextColor: control.colors.onAccent
        font.family: control.theme.fontFamily
        font.pixelSize: control.geo.bodySize
        text: control.shown
        enabled: control.field.enabled
        readOnly: !control.field.enabled
        activeFocusOnTab: false
        clip: true

        Accessible.role: Accessible.EditableText
        Accessible.name: control.field.label
        Accessible.description: control.refused ? control.field.error : control.field.disabledReason

        onTextEdited: control.report(text)
        onAccepted: control.bridge.dispatch(JSON.stringify({ type: "focus-next" }))
    }

    // The model owns the focus, so the cursor follows it rather than the other
    // way round: when the projection says this control is focused, Qt gives the
    // text cursor to it, and typing works from the keyboard alone.
    onFocusedChanged: if (focused) text.forceActiveFocus()

    function borderColour() {
        if (control.refused) {
            return control.input.borderRefused
        }
        if (control.focused) {
            return control.input.borderFocused
        }
        if (hover.hovered && control.field.enabled) {
            return control.input.borderHovered
        }

        return control.input.border
    }

    // Every edit is an action of the closed set: a cell names its row and column,
    // a number its raw text, a field its value.
    function report(text) {
        if (control.row >= 0) {
            control.bridge.dispatch(JSON.stringify({
                type: "cell-edited",
                field: control.field.field,
                row: control.row,
                column: control.column,
                text: text
            }))
            return
        }

        if (control.numeric) {
            control.bridge.dispatch(JSON.stringify({
                type: "number-edited",
                field: control.field.field,
                text: text
            }))
            return
        }

        control.bridge.dispatch(JSON.stringify({
            type: "field-set",
            field: control.field.field,
            value: { kind: "text", value: text }
        }))
    }
}
