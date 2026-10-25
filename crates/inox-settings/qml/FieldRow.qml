// An editable field of the window: its label, its control, the action that
// restores the default, and what is wrong with it.
//
// The control is drawn by one of five components, chosen by the `kind` the
// projection carries. Nothing is decided here: whether the field is enabled, why
// it is not, whether it differs from the file, and what the schema says about it
// are all values in `field`.
//
// A value the schema refuses is drawn in the danger colour and a field the
// keyboard is on is drawn with the focus ring, so a refused field that is also
// focused shows both: the colour says *what is wrong*, the width says *where the
// keyboard is* (ui-guidelines §5). A control a user cannot use always says why
// (acceptance D05).
import QtQuick
import com.inox.settings.qml

Column {
    id: row

    // One entry of the projection's fields.
    required property var field
    // The projection's theme.
    required property var theme
    // The Rust-backed element the actions are sent to.
    required property var bridge
    // The identifier of whatever the keyboard is on, so a list's cells can draw
    // the one the focus is on.
    property string focusedId: ""

    readonly property var geo: theme.geometry
    readonly property var colors: theme.colors
    readonly property bool refused: field.error !== null

    spacing: geo.rowGap

    Row {
        id: inner
        width: row.width
        spacing: row.geo.inlineGap

        Text {
            width: row.geo.labelWidth
            text: row.field.label
            color: row.field.enabled ? row.colors.text : row.colors.textMuted
            font.family: row.theme.fontFamily
            font.pixelSize: row.geo.bodySize
            wrapMode: Text.WordWrap

            Accessible.role: Accessible.StaticText
            Accessible.name: row.field.label
        }

        // The control itself: one of the five below, at the width its value
        // means.
        Loader {
            id: fieldWidth
            width: row.controlWidth
            height: item !== null ? item.implicitHeight : row.geo.controlHeight
            sourceComponent: row.componentFor(row.field.control.kind)
        }

        // The action that returns the field to the schema's default: a danger
        // action, drawn as the other discarding actions are.
        Loader {
            active: row.field.reset !== null
            sourceComponent: resetComponent
            visible: active
            height: item !== null ? item.implicitHeight : row.geo.controlHeight
        }
    }

    // What the schema says is wrong, named by the key it is about.
    Text {
        width: row.width
        visible: row.refused
        text: row.field.error !== null ? row.field.error : ""
        color: row.colors.danger
        font.family: row.theme.fontFamily
        font.pixelSize: row.geo.captionSize
        wrapMode: Text.WordWrap

        Accessible.role: Accessible.StaticText
        Accessible.name: text
    }

    // Why the control cannot be used (acceptance D05), said once per field.
    Text {
        width: row.width
        visible: !row.field.enabled && row.field.disabledReason !== null
        text: row.field.disabledReason !== null ? row.field.disabledReason : ""
        color: row.colors.textMuted
        font.family: row.theme.fontFamily
        font.pixelSize: row.geo.captionSize
        wrapMode: Text.WordWrap

        Accessible.role: Accessible.StaticText
        Accessible.name: text
    }

    // How wide the control is drawn, by what the value means: a number is as wide
    // as a number, a connector as wide as a connector, and only an explanation or
    // a table takes the row (ui-guidelines §4).
    readonly property real controlWidth: row.widthOf(row.field.width)

    function widthOf(width) {
        if (width === "short") {
            return row.geo.shortWidth
        }
        if (width === "wide") {
            return row.geo.wideWidth
        }

        return Math.max(row.width - row.geo.labelWidth - row.geo.inlineGap, row.geo.shortWidth)
    }

    function componentFor(kind) {
        if (kind === "number") {
            return numberComponent
        }
        if (kind === "choices") {
            return choicesComponent
        }
        if (kind === "flag") {
            return flagComponent
        }
        if (kind === "collection") {
            return collectionComponent
        }

        return textComponent
    }

    Component {
        id: textComponent

        TextFieldControl {
            theme: row.theme
            bridge: row.bridge
            field: row.field
            focused: row.field.focused
        }
    }

    Component {
        id: numberComponent

        NumberControl {
            theme: row.theme
            bridge: row.bridge
            field: row.field
            focused: row.field.focused
        }
    }

    Component {
        id: choicesComponent

        ChoiceRow {
            theme: row.theme
            bridge: row.bridge
            field: row.field
        }
    }

    Component {
        id: flagComponent

        FlagControl {
            theme: row.theme
            bridge: row.bridge
            field: row.field
            focused: row.field.focused
        }
    }

    Component {
        id: collectionComponent

        CollectionControl {
            theme: row.theme
            bridge: row.bridge
            field: row.field
            focusedId: row.focusedId
        }
    }

    Component {
        id: resetComponent

        ControlButton {
            theme: row.theme
            label: row.field.reset !== null ? row.field.reset : ""
            focused: row.field.resetFocused
            danger: true
            enabled: row.field.enabled
            focusId: row.field.resetFocusId
            onActivated: row.bridge.dispatch(JSON.stringify({
                type: "field-reset",
                field: row.field.field
            }))
        }
    }
}
