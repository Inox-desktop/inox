// A structured list: display outputs, a terminal command's arguments, an input
// method's arguments, per-output wallpapers.
//
// A list is a table while its columns can hold their own names; at a doubled font
// scale a seven-column table gives each column less room than the longest word in
// it, so the projection says whether to draw a header and the rows are drawn one
// labelled field per row instead (ui-guidelines §7).
//
// An empty list is a state of its own: it says what the window will do, so a user
// who sees no rows knows that is a decision rather than a failure (§6). Every row
// and cell carries the focus identifier the window's own focus uses, so walking
// the list with Tab reveals the cell the model is on.
import QtQuick
import com.inox.settings.qml

Column {
    id: list

    required property var theme
    required property var bridge
    required property var field
    // The identifier of whatever the keyboard is on.
    property string focusedId: ""

    readonly property var geo: theme.geometry
    readonly property var colors: theme.colors
    readonly property var value: field.control

    spacing: geo.rowGap

    Column {
        id: content
        width: list.width
        spacing: list.geo.rowGap

        Text {
            width: parent.width
            text: list.value.help
            color: list.colors.textMuted
            font.family: list.theme.fontFamily
            font.pixelSize: list.geo.captionSize
            wrapMode: Text.WordWrap

            Accessible.role: Accessible.StaticText
            Accessible.name: list.value.help
        }

        // The header line, when the list is drawn as a table.
        Row {
            width: parent.width
            spacing: list.geo.inlineGap
            visible: list.value.asTable && !list.value.isEmpty
            height: visible ? header.implicitHeight : 0

            Row {
                id: header
                width: parent.width - list.geo.listActionWidth - list.geo.inlineGap
                spacing: list.geo.inlineGap

                Repeater {
                    model: list.value.columns

                    delegate: Text {
                        required property var modelData
                        required property int index
                        width: (header.width - (list.value.columns.length - 1) * list.geo.inlineGap)
                               * list.weightOf(index)
                        text: modelData
                        color: list.colors.textMuted
                        font.family: list.theme.fontFamily
                        font.pixelSize: list.geo.captionSize
                        wrapMode: Text.WordWrap

                        Accessible.role: Accessible.StaticText
                        Accessible.name: modelData
                    }
                }
            }
        }

        // What an empty list means, in words.
        Text {
            width: parent.width
            visible: list.value.isEmpty
            text: list.value.empty
            color: list.colors.textMuted
            font.family: list.theme.fontFamily
            font.pixelSize: list.geo.captionSize
            wrapMode: Text.WordWrap

            Accessible.role: Accessible.StaticText
            Accessible.name: list.value.empty
        }

        Repeater {
            model: list.value.rows

            delegate: Row {
                id: rowItem
                required property var modelData
                required property int index
                width: content.width
                spacing: list.geo.inlineGap
                height: rowContent.implicitHeight

                Row {
                    id: rowContent
                    width: rowItem.width - list.geo.listActionWidth - list.geo.inlineGap
                    spacing: list.geo.inlineGap

                    Repeater {
                        model: rowItem.modelData.cells

                        delegate: TextFieldControl {
                            required property var modelData
                            required property int index
                            width: (rowContent.width - (list.value.columns.length - 1) * list.geo.inlineGap)
                                   * list.weightOf(index)
                            theme: list.theme
                            bridge: list.bridge
                            field: list.field
                            row: rowItem.index
                            column: index
                            focused: list.focusedId === modelData.focusId
                        }
                    }
                }

                ControlButton {
                    theme: list.theme
                    label: rowItem.modelData.remove
                    danger: true
                    focused: rowItem.modelData.removeFocused
                    enabled: list.field.enabled
                    width: list.geo.listActionWidth
                    focusId: rowItem.modelData.removeFocusId
                    onActivated: list.bridge.dispatch(JSON.stringify({
                        type: "row-removed",
                        field: list.field.field,
                        row: rowItem.index
                    }))
                }
            }
        }

        ControlButton {
            theme: list.theme
            label: list.value.add
            focused: list.value.addFocused
            enabled: list.field.enabled
            focusId: list.value.addFocusId
            onActivated: list.bridge.dispatch(JSON.stringify({
                type: "row-added",
                field: list.field.field
            }))
        }
    }

    // How much of the row one column takes, relative to the others. The weights
    // come from the projection, so a connector is roomier than a number and no
    // column is ever drawn as wide as the window (ui-guidelines §4).
    function weightOf(column) {
        var rows = list.value.rows

        if (rows.length === 0) {
            return 1 / Math.max(list.value.columns.length, 1)
        }

        var cells = rows[0].cells
        var total = 0

        for (var index = 0; index < cells.length; ++index) {
            total += cells[index].weight
        }

        if (total === 0 || column >= cells.length) {
            return 1 / Math.max(cells.length, 1)
        }

        return cells[column].weight / total
    }
}
