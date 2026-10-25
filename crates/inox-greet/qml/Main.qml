// The Inox login screen.
//
// This tree only renders. Every colour, every distance and every user-visible
// string comes from the Rust-backed `Greeter` element in `src/view.rs` and
// `src/cxxqt_object.rs`, which projects the design tokens and the Fluent catalogs
// (root AGENTS.md §4 and §7.3). Do not add colour literals, pixel sizes or
// user-visible text here — add them to `src/view.rs` instead.
//
// The password is not here. There is no `TextInput` and no `TextField` on this
// screen: the field below draws a mask the projection built from the credential's
// *length*, and the keys that fill it are reported to Rust one at a time. What the
// person types lives in `inox_common::secret::Secret` and never crosses back into
// QML.
import QtQuick
import QtQuick.Window
import com.inox.greet.qml

Window {
    id: root

    // The whole projection: window geometry, tokens, and the current interface
    // state. Re-sent by Rust on every change, so every binding below restates
    // itself from one object rather than from a property per field.
    readonly property var build: JSON.parse(greeter.modelJson)
    readonly property var ui: build.ui
    readonly property var colors: build.colors
    readonly property var fonts: build.fonts
    readonly property var space: build.space
    readonly property var radius: build.radius
    readonly property var metrics: build.metrics
    readonly property var layout: build.layout
    readonly property var focusRing: build.focusRing

    // The card is a proportion of the window and the controls a proportion of the
    // card (guidelines §4, ). The proportions are Rust's; the arithmetic
    // is here because it is arithmetic, not a design decision.
    readonly property real cardWidth: Math.round(width * layout.cardPortion
            / (layout.cardPortion + 2 * layout.marginPortion))
    readonly property real controlWidth: Math.round((cardWidth - 2 * space.lg)
            * layout.controlPortion
            / (layout.controlPortion + 2 * layout.controlGutterPortion))

    width: build.window.width
    height: build.window.height
    visible: true
    title: build.window.title
    color: build.window.background

    // The Rust-backed state machine. Its `modelJson` property is the projection
    // re-sent on every change; its invokables are the whole way back.
    Greeter {
        id: greeter
    }

    // One line of text the projection decided: what it says, which token colour it
    // is drawn in, and which type role sizes it. An absent line is an invisible
    // item, which is also what keeps it out of the semantic tree.
    component Line: Text {
        property var line

        width: parent ? parent.width : implicitWidth
        visible: line !== null && line !== undefined
        text: visible ? line.text : ""
        color: root.colors[visible ? line.color : "text"]
        font.family: root.fonts.family
        font.pixelSize: root.fonts[visible ? line.size : "body"]
        wrapMode: Text.WordWrap

        Accessible.role: Accessible.StaticText
        Accessible.name: text
    }

    // The screen's one primary action: filled with the accent while it can be used,
    // and reduced to muted text with no fill when it cannot (guidelines §5,
    // acceptance D05 — the reason is drawn beside it). The fill does not follow the
    // pointer; a hover is answered by an on-accent hairline, and the focus ring is
    // the same colour drawn one `spacing.xs` wide.
    component PrimaryControl: Rectangle {
        property string label
        property bool enabled: true
        property bool focused: false
        property string action

        width: root.controlWidth
        height: root.fonts.body + 2 * root.metrics.controlPadding
        radius: root.radius.sm
        color: enabled ? root.colors.accent : "transparent"
        border.width: enabled && (focused || hover.hovered) ? root.metrics.focusRing : 0
        border.color: root.focusRing.onAccent

        Text {
            anchors.centerIn: parent
            text: parent.label
            color: parent.enabled ? root.colors.onAccent : root.colors.textMuted
            font.family: root.fonts.family
            font.pixelSize: root.fonts.body
        }

        HoverHandler {
            id: hover
            enabled: parent.enabled
        }

        MouseArea {
            anchors.fill: parent
            enabled: parent.enabled
            cursorShape: Qt.PointingHandCursor
            onClicked: greeter.dispatch(parent.action)
        }

        Accessible.role: Accessible.Button
        Accessible.name: label
        Accessible.focusable: enabled
    }

    // Any control that is not the primary action: text on a hairline outline at
    // rest, the alternate surface under the pointer, the accent while it is being
    // pressed, and the accent ring where the keyboard is (guidelines §5).
    component ActionControl: Rectangle {
        property string label
        property bool enabled: true
        property bool focused: false
        property string action

        width: root.controlWidth
        height: root.fonts.body + 2 * root.metrics.controlPadding
        radius: root.radius.sm
        color: pointer.pressed ? root.colors.accent
                : hover.hovered ? root.colors.surfaceVariant
                : "transparent"
        border.width: !enabled ? 0 : focused ? root.metrics.focusRing : root.metrics.hairline
        border.color: focused ? root.focusRing.onSurface : root.colors.border

        Text {
            anchors.centerIn: parent
            text: parent.label
            color: pointer.pressed ? root.colors.onAccent
                    : parent.enabled ? root.colors.text
                    : root.colors.textMuted
            font.family: root.fonts.family
            font.pixelSize: root.fonts.body
        }

        HoverHandler {
            id: hover
            enabled: parent.enabled
        }

        MouseArea {
            id: pointer
            anchors.fill: parent
            enabled: parent.enabled
            cursorShape: Qt.PointingHandCursor
            onClicked: greeter.dispatch(parent.action)
        }

        Accessible.role: Accessible.Button
        Accessible.name: label
        Accessible.focusable: enabled
    }

    Timer {
        // The answers `auth-v1` delivered arrive on the boundary's own threads, so
        // the tree asks for them. A login screen has nothing to animate: ten times a
        // second is imperceptible for a reply and does not keep a core awake.
        interval: 100
        running: true
        repeat: true
        onTriggered: greeter.pump()
    }

    Timer {
        // The model's clock: the reply deadline, the rolling allowance and the
        // started face's dwell all advance on this and on nothing else.
        interval: 1000
        running: true
        repeat: true
        onTriggered: greeter.tick()
    }

    // Everything the screen says lives here, so the semantic tree has one root and
    // the keyboard has one owner.
    FocusScope {
        id: content
        anchors.fill: parent
        focus: true

        Accessible.role: Accessible.Pane
        Accessible.name: root.build.window.title

        // QML reports keys; it does not interpret them (docs/qml-migration-contract.md
        // §6). Space is absent on purpose: it is a character somebody may want in
        // their own password, so it goes through `greeter.text` like any other.
        Keys.onPressed: function (event) {
            let handled = true
            if (event.key === Qt.Key_Tab) {
                greeter.key(event.modifiers & Qt.ShiftModifier ? "shift-tab" : "tab")
            } else if (event.key === Qt.Key_Backtab) {
                greeter.key("shift-tab")
            } else if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter) {
                greeter.key("enter")
            } else if (event.key === Qt.Key_Escape) {
                greeter.key("escape")
            } else if (event.key === Qt.Key_Backspace) {
                greeter.key("backspace")
            } else if (event.text.length > 0) {
                greeter.text(event.text)
            } else {
                handled = false
            }
            event.accepted = handled
        }

        // Centred while it fits and scrolled when it does not, which is what keeps
        // 200 % text readable instead of clipped (acceptance Q02).
        Flickable {
            id: viewport
            anchors.fill: parent
            anchors.margins: root.space.xl
            clip: true
            contentWidth: width
            contentHeight: Math.max(height, card.height)
            boundsBehavior: Flickable.StopAtBounds

            Rectangle {
                id: card
                width: root.cardWidth
                height: cardColumn.implicitHeight + 2 * root.space.lg
                x: Math.round((viewport.width - width) / 2)
                y: Math.round((viewport.contentHeight - height) / 2)
                radius: root.radius.md
                color: root.colors.surface
                border.width: root.metrics.hairline
                border.color: root.colors.border

                Column {
                    id: cardColumn
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.verticalCenter: parent.verticalCenter
                    anchors.margins: root.space.lg
                    spacing: root.space.lg
                    width: parent.width - 2 * root.space.lg

                    // ---- The accounts to log in as ----------------
                    Column {
                        id: accountsPane
                        visible: root.ui.choosing
                        width: parent.width
                        spacing: root.space.sm

                        Line {
                            id: accountsHeading
                            line: root.ui.accountsHeading
                        }

                        Repeater {
                            model: root.ui.accounts

                            delegate: Rectangle {
                                id: accountRow
                                required property var modelData
                                required property int index

                                width: parent ? parent.width : 0
                                height: root.fonts.body + 2 * root.metrics.controlPadding
                                radius: root.radius.sm
                                color: modelData.selected || rowPointer.pressed
                                        ? root.colors.accent
                                        : rowHover.hovered ? root.colors.surfaceVariant
                                        : "transparent"
                                border.width: modelData.focused ? root.metrics.focusRing
                                        : root.metrics.hairline
                                border.color: modelData.focused
                                        ? (modelData.selected
                                                ? root.focusRing.onAccent
                                                : root.focusRing.onSurface)
                                        : modelData.selected
                                                ? "transparent"
                                                : root.colors.border

                                Rectangle {
                                    id: rowMedallion
                                    width: root.metrics.rowMedallion
                                    height: root.metrics.rowMedallion
                                    radius: width / 2
                                    anchors.left: parent.left
                                    anchors.verticalCenter: parent.verticalCenter
                                    anchors.leftMargin: root.metrics.controlPadding
                                    color: root.colors.surfaceVariant
                                    border.width: root.metrics.hairline
                                    border.color: root.colors.border

                                    Text {
                                        anchors.centerIn: parent
                                        text: accountRow.modelData.initial
                                        color: root.colors.text
                                        font.family: root.fonts.family
                                        font.pixelSize: root.fonts.body
                                    }
                                }

                                Text {
                                    anchors.left: rowMedallion.right
                                    anchors.right: parent.right
                                    anchors.verticalCenter: parent.verticalCenter
                                    anchors.leftMargin: root.space.md
                                    anchors.rightMargin: root.metrics.controlPadding
                                    text: accountRow.modelData.label
                                    elide: Text.ElideRight
                                    color: accountRow.modelData.selected
                                            ? root.colors.onAccent
                                            : root.colors.text
                                    font.family: root.fonts.family
                                    font.pixelSize: root.fonts.body
                                }

                                HoverHandler {
                                    id: rowHover
                                }

                                MouseArea {
                                    id: rowPointer
                                    anchors.fill: parent
                                    cursorShape: Qt.PointingHandCursor
                                    onClicked: greeter.dispatch(
                                            '{"type":"select-account","index":'
                                            + accountRow.index + '}')
                                }

                                Accessible.role: Accessible.Button
                                Accessible.name: modelData.label
                                Accessible.description: modelData.selected
                                        ? accountRow.modelData.name : ""
                            }
                        }

                        ActionControl {
                            id: backControl
                            anchors.horizontalCenter: parent.horizontalCenter
                            label: root.ui.cancel.label
                            enabled: root.ui.cancel.enabled
                            focused: root.ui.cancel.focused
                            action: '{"type":"cancel"}'
                        }
                    }

                    // ---- The login itself ------------------------------------
                    Column {
                        id: loginPane
                        visible: !root.ui.choosing
                        width: parent.width
                        spacing: root.space.lg

                        // The account a login would use: its medallion and its name.
                        Column {
                            id: who
                            visible: root.ui.selected !== null
                            width: parent.width
                            spacing: root.space.sm

                            Rectangle {
                                id: medallion
                                width: root.metrics.medallion
                                height: root.metrics.medallion
                                radius: width / 2
                                anchors.horizontalCenter: parent.horizontalCenter
                                color: root.colors.surfaceVariant
                                border.width: root.metrics.hairline
                                border.color: root.colors.border

                                Text {
                                    anchors.centerIn: parent
                                    text: root.ui.selected !== null
                                            ? root.ui.selected.initial : ""
                                    color: root.colors.text
                                    font.family: root.fonts.family
                                    font.pixelSize: root.fonts.title
                                }

                                Accessible.role: Accessible.StaticText
                                Accessible.name: root.ui.selected !== null
                                        ? root.ui.selected.name : ""
                            }

                            Text {
                                width: parent.width
                                horizontalAlignment: Text.AlignHCenter
                                text: root.ui.selected !== null
                                        ? root.ui.selected.label : ""
                                color: root.colors.text
                                font.family: root.fonts.family
                                font.pixelSize: root.fonts.title
                                wrapMode: Text.WordWrap

                                Accessible.role: Accessible.Heading
                                Accessible.name: text
                            }
                        }

                        // Why there is nobody to log in as, when there is nobody.
                        Line {
                            id: accountsReason
                            line: root.ui.accountsReason
                        }

                        // What this build is. A preview collects nothing and starts
                        // nothing; a build with a boundary behind it does both.
                        Line {
                            id: banner
                            line: root.ui.banner
                        }

                        // The password field. It draws the mask the projection built
                        // and takes keys; it holds no value.
                        Rectangle {
                            id: credentialField
                            width: root.controlWidth
                            height: credentialColumn.implicitHeight
                                    + 2 * root.metrics.controlPadding
                            anchors.horizontalCenter: parent.horizontalCenter
                            radius: root.radius.sm
                            color: root.ui.credential.editable
                                    ? root.colors.background : "transparent"
                            border.width: !root.ui.credential.editable ? 0
                                    : root.ui.credential.focused
                                            ? root.metrics.focusRing : root.metrics.hairline
                            border.color: !root.ui.credential.editable
                                    ? "transparent"
                                    : root.ui.credential.focused
                                            ? root.focusRing.onBackground
                                            : root.ui.credential.error !== null
                                                    ? root.colors.danger
                                                    : root.colors.border

                            Column {
                                id: credentialColumn
                                anchors.left: parent.left
                                anchors.right: parent.right
                                anchors.verticalCenter: parent.verticalCenter
                                anchors.margins: root.metrics.controlPadding
                                spacing: root.space.sm

                                Line {
                                    id: credentialLabel
                                    line: root.ui.credential.label
                                }

                                Line {
                                    id: credentialBody
                                    line: root.ui.credential.body
                                }
                            }

                            Accessible.role: Accessible.EditableText
                            Accessible.name: root.ui.credential.label.text
                            Accessible.description: root.ui.credential.disabledReason !== null
                                    ? root.ui.credential.disabledReason.text : ""
                            Accessible.focusable: root.ui.credential.editable
                        }

                        // What is wrong with what the screen was handed.
                        Line {
                            id: credentialError
                            line: root.ui.credential.error
                        }

                        // What the login is doing: the state under the account name.
                        Line {
                            id: faceLine
                            line: root.ui.face.line
                        }

                        Line {
                            id: faceReason
                            line: root.ui.face.reason
                        }

                        // The one line that says what else is in the way.
                        Line {
                            id: notice
                            line: root.ui.notice
                        }

                        PrimaryControl {
                            id: submitControl
                            anchors.horizontalCenter: parent.horizontalCenter
                            label: root.ui.submit.label
                            enabled: root.ui.submit.enabled
                            focused: root.ui.submit.focused
                            action: '{"type":"submit"}'
                        }

                        Line {
                            id: submitReason
                            line: root.ui.submit.reason
                        }

                        // The secondary controls: switching account where there is a
                        // choice, and clearing the password — or, with the account
                        // list open, leaving it.
                        Row {
                            id: secondaryRow
                            width: root.controlWidth
                            anchors.horizontalCenter: parent.horizontalCenter
                            spacing: root.space.sm

                            ActionControl {
                                id: switchControl
                                visible: root.ui.switchUser !== null
                                width: visible
                                        ? (secondaryRow.width - secondaryRow.spacing) / 2
                                        : 0
                                label: visible ? root.ui.switchUser.label : ""
                                enabled: visible && root.ui.switchUser.enabled
                                focused: visible && root.ui.switchUser.focused
                                action: '{"type":"switch-user"}'
                            }

                            ActionControl {
                                id: cancelControl
                                width: switchControl.visible
                                        ? (secondaryRow.width - secondaryRow.spacing) / 2
                                        : secondaryRow.width
                                label: root.ui.cancel.label
                                enabled: root.ui.cancel.enabled
                                focused: root.ui.cancel.focused
                                action: '{"type":"cancel"}'
                            }
                        }

                        Line {
                            id: hint
                            line: root.ui.hint
                        }

                        // What a login would start. The label and the session read as
                        // one line because they are one fact.
                        Row {
                            width: parent.width
                            spacing: root.space.sm

                            Line {
                                id: sessionLabel
                                width: implicitWidth
                                line: root.ui.sessionLabel
                            }

                            Line {
                                id: sessionName
                                width: implicitWidth
                                line: root.ui.session
                            }
                        }
                    }
                }
            }
        }
    }

    // The headless evidence (`--preview --self-check`): the tree has built, it read
    // the Rust projection, and it carries the semantic tree a screen reader walks
    // (root AGENTS.md §7.4). `reportReady` only accepts the report when the title
    // and the accessible-item count agree with the projection.
    Component.onCompleted: function () {
        content.forceActiveFocus()
        greeter.reportReady(root.build.window.title, root.accessibleItemCount(content))
    }

    // Count the items that expose an accessibility role, so the headless check can
    // assert the semantic tree exists instead of trusting that it does. An invisible
    // subtree is skipped whole, which is why an absent line costs nothing.
    function accessibleItemCount(item) {
        if (item === null || item === undefined || item.visible === false)
            return 0

        let count = 0
        if (item.Accessible !== undefined && item.Accessible.role !== Accessible.NoRole)
            count += 1

        let children = item.children || []
        for (let i = 0; i < children.length; ++i)
            count += root.accessibleItemCount(children[i])

        return count
    }
}
