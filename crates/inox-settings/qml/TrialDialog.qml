// The question a display change asks before it becomes permanent.
//
// A display change is applied live but only for as long as the user does not
// answer: the compositor holds a trial and reverts it itself, and this window
// counts the same seconds down, asks in a dialog, and restores the previous
// settings when the answer is "no" or nobody answers. It is drawn on top of the
// window rather than in a place of its own: the display has already changed, so
// the window is exactly what the user may be losing.
//
// The scrim, the card's outline and both buttons' states come from the tokens;
// the countdown is the number the state machine decrements, drawn as the sentence
// the user reads.
import QtQuick
import com.inox.settings.qml

Item {
    id: dialog

    required property var trial
    required property var theme
    required property var bridge

    readonly property var geo: theme.geometry
    readonly property var colors: theme.colors

    // The dialog is built with the window and shown only while a display trial is
    // open, so every binding below reads through this: `trial` is null the rest of
    // the time, and binding to `trial.title` directly would raise a TypeError on
    // every projection. The placeholder is drawn never and says nothing; it exists
    // so the item tree is the same in both states.
    readonly property var shown: trial !== null
                                 ? trial
                                 : ({ title: "", message: "", error: null, choices: [] })

    Rectangle {
        anchors.fill: parent
        color: dialog.colors.scrim
    }

    Rectangle {
        anchors.centerIn: parent
        width: dialog.geo.dialogWidth
        implicitHeight: card.implicitHeight + 2 * dialog.geo.cardPadding
        color: dialog.colors.surface
        radius: dialog.geo.cardRadius
        border.width: dialog.geo.hairline
        border.color: dialog.colors.dialogBorder

        Column {
            id: card
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.top: parent.top
            anchors.margins: dialog.geo.cardPadding
            spacing: dialog.geo.rowGap

            Text {
                width: parent.width
                text: dialog.shown.title
                color: dialog.colors.text
                font.family: dialog.theme.fontFamily
                font.pixelSize: dialog.geo.titleSize
                font.weight: dialog.geo.emphasisWeight
                wrapMode: Text.WordWrap

                Accessible.role: Accessible.Heading
                Accessible.name: dialog.shown.title
            }

            Text {
                width: parent.width
                text: dialog.shown.message
                color: dialog.colors.text
                font.family: dialog.theme.fontFamily
                font.pixelSize: dialog.geo.bodySize
                wrapMode: Text.WordWrap

                Accessible.role: Accessible.StaticText
                Accessible.name: dialog.shown.message
            }

            Text {
                width: parent.width
                visible: dialog.shown.error !== null
                text: dialog.shown.error !== null ? dialog.shown.error : ""
                color: dialog.colors.danger
                font.family: dialog.theme.fontFamily
                font.pixelSize: dialog.geo.captionSize
                wrapMode: Text.WordWrap

                Accessible.role: Accessible.StaticText
                Accessible.name: text
            }

            Row {
                width: parent.width
                spacing: dialog.geo.inlineGap

                Repeater {
                    model: dialog.shown.choices

                    delegate: Column {
                        required property var modelData
                        spacing: dialog.geo.rowGap

                        ControlButton {
                            theme: dialog.theme
                            label: modelData.label
                            focused: modelData.focused
                            enabled: modelData.enabled
                            danger: !modelData.confirm
                            focusId: modelData.focusId
                            onActivated: dialog.bridge.dispatch(JSON.stringify({
                                type: modelData.confirm ? "trial-confirm" : "trial-revert"
                            }))
                        }

                        Text {
                            width: parent.width
                            visible: modelData.disabledReason !== null
                            text: modelData.disabledReason !== null ? modelData.disabledReason : ""
                            color: dialog.colors.textMuted
                            font.family: dialog.theme.fontFamily
                            font.pixelSize: dialog.geo.captionSize
                            wrapMode: Text.WordWrap
                        }
                    }
                }
            }
        }
    }
}
