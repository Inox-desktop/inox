// The status line: what happened to the change, and what each consumer answered.
//
// It is always there, and it never claims more than the Rust state machine knows:
// saved-but-not-confirmed and in-effect are different sentences in different
// colours, and a display trial that is still waiting is
// drawn in the colour that says so. The colour comes from the
// projection, which took it from the tokens.
import QtQuick

Rectangle {
    id: strip

    required property var status
    required property var theme

    readonly property var geo: theme.geometry
    readonly property var colors: theme.colors

    implicitHeight: column.implicitHeight + 2 * geo.noticePadding
    color: colors.surface
    radius: geo.cardRadius
    border.width: geo.hairline
    border.color: status.border

    Column {
        id: column
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        anchors.margins: strip.geo.noticePadding
        spacing: strip.geo.rowGap

        Text {
            width: parent.width
            text: strip.status.message
            color: strip.colors.text
            font.family: strip.theme.fontFamily
            font.pixelSize: strip.geo.bodySize
            wrapMode: Text.WordWrap

            Accessible.role: Accessible.StaticText
            Accessible.name: strip.status.message
        }

        Repeater {
            model: strip.status.details

            delegate: Text {
                required property var modelData
                width: column.width
                text: modelData
                color: strip.colors.textMuted
                font.family: strip.theme.fontFamily
                font.pixelSize: strip.geo.captionSize
                wrapMode: Text.WordWrap

                Accessible.role: Accessible.StaticText
                Accessible.name: modelData
            }
        }
    }
}
