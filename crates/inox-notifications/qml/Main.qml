// The Inox notification popup stack.
//
// This tree only renders. Every colour, distance, radius, font size and
// user-visible sentence comes from the Rust-backed `Notifications` element below,
// which projects the daemon's state, the design tokens and the Fluent catalogs
// into one JSON model (`src/view.rs`). Do not add a colour, a pixel size or a
// piece of user-visible text here — add it to the projection instead (root
// AGENTS.md §4 and §7.3, docs/qml-migration-contract.md §4).
//
// The daemon's state machine, queue, policy and lock-state redaction all live in
// Rust. This surface reports three things back and nothing else:
//   * `pump()` — the frame timer that drains what the D-Bus transport delivered
//     on its own thread and refreshes the model when something changed;
//   * `dispatch(json)` — one action the user took (close / action / invoke / link);
//   * `reportReady(...)` — the load evidence the headless self-check waits for.
import QtQuick
import QtQuick.Window
import com.inox.notifications.qml

Window {
    id: root

    // The whole interface, projected in Rust. QML never computes a colour, a size
    // or a sentence of its own: it reads this value and draws it.
    property var vm: JSON.parse(notifications.modelJson)

    // The number of popups the projection carries, kept for the self-check's
    // report: the tree has to instantiate every one of them.
    property int popupCards: 0

    width: vm.window.width
    height: vm.window.height
    title: vm.window.title
    color: vm.palette.background
    visible: true

    // The Rust-backed daemon bridge: `modelJson` plus the three invokables above.
    Notifications {
        id: notifications
    }

    // The frame pump. `pump()` drains the inbox the transports write into on
    // their own threads; it runs on the Qt thread, so no QML object is ever
    // touched from a background thread (docs/qml-migration-contract.md §3).
    Timer {
        interval: 16
        running: true
        repeat: true
        onTriggered: notifications.pump()
    }

    // Confirms the loaded tree once, after the first turn of the event loop so
    // that repeaters have instantiated their delegates.
    Timer {
        id: readyReporter

        interval: 0
        running: true
        repeat: false
        onTriggered: {
            // Count what the surface actually built, then let Rust compare it
            // with the projection it handed over.
            root.popupCards = root.countPopupCards(stack)
            notifications.reportReady(root.title, root.countAccessibleItems(stack), root.popupCards)
        }
    }

    // Counts the items that expose an accessibility role, so the headless check
    // can assert the semantic tree exists instead of trusting that it does
    // (root AGENTS.md §7.4).
    function countAccessibleItems(item) {
        if (item === null || item === undefined || item.children === undefined) {
            return 0
        }

        var exposed = item.Accessible !== undefined
                && item.Accessible.role !== undefined
                && item.Accessible.role !== Accessible.NoRole ? 1 : 0
        var children = item.children

        for (var i = 0; i < children.length; ++i) {
            exposed += root.countAccessibleItems(children[i])
        }

        return exposed
    }

    // Counts the popup cards the tree instantiated, so a repeater that produced
    // nothing cannot pass the check.
    function countPopupCards(item) {
        if (item === null || item === undefined || item.children === undefined) {
            return 0
        }

        var cards = item.popupCard === true ? 1 : 0
        var children = item.children

        for (var i = 0; i < children.length; ++i) {
            cards += root.countPopupCards(children[i])
        }

        return cards
    }

    // The stack surface: the popups sit in the configured corner, inset by the
    // configured margin, which is the same place the layer-surface anchors would
    // put them once this component has a layer-shell host.
    FocusScope {
        id: stack

        anchors.fill: parent
        anchors.margins: vm.metrics.margin

        Accessible.role: Accessible.Pane
        Accessible.name: vm.window.title

        Column {
            id: column

            width: Math.min(stack.width, vm.metrics.width)
            spacing: vm.metrics.cardGap

            anchors.left: vm.alignment.horizontal === "left" ? parent.left : undefined
            anchors.right: vm.alignment.horizontal === "right" ? parent.right : undefined
            anchors.top: vm.alignment.vertical === "top" ? parent.top : undefined
            anchors.bottom: vm.alignment.vertical === "bottom" ? parent.bottom : undefined

            // One card per notification, newest first — the order is the
            // projection's, not the surface's.
            Repeater {
                model: vm.stack.popups

                delegate: Rectangle {
                    id: card

                    // Read by the self-check: the tree really instantiated this
                    // popup, rather than merely being told about it.
                    property bool popupCard: true
                    property var popup: modelData

                    width: column.width
                    height: cardColumn.implicitHeight + 2 * vm.metrics.cardInset

                    color: popup.card.background
                    border.color: popup.card.edge
                    border.width: vm.metrics.borderWidth
                    radius: popup.card.radius

                    Accessible.role: Accessible.Pane
                    Accessible.name: popup.summary

                    Column {
                        id: cardColumn

                        x: vm.metrics.cardInset
                        y: vm.metrics.cardInset
                        width: parent.width - 2 * vm.metrics.cardInset
                        spacing: vm.metrics.sectionGap

                        // The header: the application's icon's place, who is
                        // speaking, and the close button.
                        Item {
                            width: parent.width
                            height: Math.max(headerRow.height, closeButton.height)

                            Row {
                                id: headerRow

                                anchors.left: parent.left
                                anchors.verticalCenter: parent.verticalCenter
                                spacing: vm.metrics.inlineGap

                                // The space an application icon occupies. This
                                // daemon does not decode icons yet, so it draws
                                // the placeholder rather than a hole.
                                Rectangle {
                                    visible: popup.icon.present
                                    width: popup.icon.size
                                    height: popup.icon.size
                                    radius: popup.icon.radius
                                    color: popup.icon.fill
                                }

                                Text {
                                    visible: popup.appName !== null
                                    text: popup.appName !== null ? popup.appName : ""
                                    color: vm.palette.textMuted
                                    font.pixelSize: vm.metrics.captionSize

                                    Accessible.role: Accessible.StaticText
                                    Accessible.name: text
                                }
                            }

                            Rectangle {
                                id: closeButton

                                property bool hovered: closeArea.containsMouse
                                property bool pressed: closeArea.pressed
                                property var control: popup.close.control

                                anchors.right: parent.right
                                anchors.verticalCenter: parent.verticalCenter
                                width: closeLabel.implicitWidth + 2 * vm.metrics.controlInset
                                height: vm.metrics.controlHeight
                                radius: vm.metrics.controlRadius

                                color: pressed ? control.press.fill
                                                : hovered ? control.hover.fill
                                                          : control.rest.fill
                                border.width: vm.metrics.borderWidth
                                border.color: pressed ? control.press.border
                                                      : hovered ? control.hover.border
                                                                : control.rest.border

                                Text {
                                    id: closeLabel

                                    anchors.centerIn: parent
                                    text: popup.close.label
                                    color: pressed ? control.press.text
                                                   : hovered ? control.hover.text
                                                             : control.rest.text
                                    font.pixelSize: vm.metrics.bodySize
                                }

                                Accessible.role: Accessible.Button
                                Accessible.name: popup.close.label

                                MouseArea {
                                    id: closeArea

                                    anchors.fill: parent
                                    hoverEnabled: true
                                    cursorShape: Qt.PointingHandCursor
                                    onClicked: notifications.dispatch(JSON.stringify({
                                        "type": "close",
                                        "id": popup.id
                                    }))
                                }
                            }
                        }

                        // The card's one heading: the notification's summary.
                        Text {
                            width: parent.width
                            text: popup.summary
                            color: vm.palette.text
                            font.pixelSize: vm.metrics.headingSize
                            font.bold: true
                            wrapMode: Text.WrapAtWordBoundaryOrAnywhere

                            Accessible.role: Accessible.Heading
                            Accessible.name: popup.summary
                        }

                        // The body: the spans the sanitiser produced, each one
                        // drawn with the weight, style, underline and colour the
                        // projection resolved. A link is the only span that does
                        // anything when clicked.
                        Item {
                            id: bodyBox

                            width: parent.width
                            height: bodyFlow.height
                            visible: popup.body.length > 0

                            Accessible.role: Accessible.StaticText
                            Accessible.name: popup.bodyText

                            Flow {
                                id: bodyFlow

                                width: parent.width
                                spacing: 0

                                Repeater {
                                    model: popup.body

                                    delegate: Text {
                                        id: span

                                        property var piece: modelData

                                        text: piece.text
                                        color: piece.color
                                        font.pixelSize: vm.metrics.bodySize
                                        font.bold: piece.bold
                                        font.italic: piece.italic
                                        font.underline: piece.underline
                                        wrapMode: Text.WrapAtWordBoundaryOrAnywhere

                                        MouseArea {
                                            anchors.fill: parent
                                            enabled: span.piece.link
                                            hoverEnabled: span.piece.link
                                            cursorShape: span.piece.link ? Qt.PointingHandCursor : Qt.ArrowCursor
                                            onClicked: notifications.dispatch(JSON.stringify({
                                                "type": "link",
                                                "id": popup.id,
                                                "href": span.piece.href
                                            }))
                                        }
                                    }
                                }
                            }
                        }

                        // The actions, laid out in rows of at most
                        // `actionsPerRow` so that even sixteen actions wrap
                        // instead of running off the card (ui-guidelines §7).
                        Flow {
                            width: parent.width
                            spacing: vm.metrics.inlineGap
                            visible: popup.actions.length > 0

                            Repeater {
                                model: popup.actions

                                delegate: Rectangle {
                                    id: actionButton

                                    property var action: modelData
                                    property bool hovered: actionArea.containsMouse
                                    property bool pressed: actionArea.pressed

                                    width: (cardColumn.width
                                            - (vm.metrics.actionsPerRow - 1) * vm.metrics.controlGap)
                                           / vm.metrics.actionsPerRow
                                    height: vm.metrics.controlHeight
                                    radius: vm.metrics.controlRadius

                                    color: !action.enabled ? action.control.disabled.fill
                                           : pressed ? action.control.press.fill
                                                     : hovered ? action.control.hover.fill
                                                               : action.control.rest.fill
                                    border.width: vm.metrics.borderWidth
                                    border.color: !action.enabled ? action.control.disabled.border
                                                  : pressed ? action.control.press.border
                                                            : hovered ? action.control.hover.border
                                                                      : action.control.rest.border

                                    Text {
                                        anchors.centerIn: parent
                                        width: parent.width - 2 * vm.metrics.controlInset
                                        text: actionButton.action.label
                                        color: !actionButton.action.enabled ? actionButton.action.control.disabled.text
                                               : actionButton.pressed ? actionButton.action.control.press.text
                                                                      : actionButton.hovered ? actionButton.action.control.hover.text
                                                                                             : actionButton.action.control.rest.text
                                        font.pixelSize: vm.metrics.bodySize
                                        horizontalAlignment: Text.AlignHCenter
                                        wrapMode: Text.WrapAtWordBoundaryOrAnywhere
                                    }

                                    Accessible.role: Accessible.Button
                                    Accessible.name: action.label
                                    Accessible.description: action.disabledReason !== null ? action.disabledReason : ""

                                    MouseArea {
                                        id: actionArea

                                        anchors.fill: parent
                                        enabled: actionButton.action.enabled
                                        hoverEnabled: true
                                        cursorShape: actionButton.action.enabled ? Qt.PointingHandCursor : Qt.ArrowCursor
                                        onClicked: notifications.dispatch(JSON.stringify({
                                            "type": "action",
                                            "id": popup.id,
                                            "key": actionButton.action.id
                                        }))
                                    }
                                }
                            }
                        }

                        // Why every action of this notification is unavailable,
                        // said on the card rather than only on hover (D05).
                        Text {
                            width: parent.width
                            visible: popup.actions.length > 0 && popup.actions[0].disabledReason !== null
                            text: popup.actions.length > 0 && popup.actions[0].disabledReason !== null
                                  ? popup.actions[0].disabledReason : ""
                            color: vm.palette.textMuted
                            font.pixelSize: vm.metrics.captionSize
                            wrapMode: Text.WrapAtWordBoundaryOrAnywhere

                            Accessible.role: Accessible.StaticText
                            Accessible.name: text
                        }

                        // The status line: how urgent it is (as a sentence, not
                        // only as a colour) and how long ago it arrived.
                        Row {
                            width: parent.width
                            spacing: vm.metrics.inlineGap

                            Text {
                                visible: popup.urgencyNote !== null
                                text: popup.urgencyNote !== null ? popup.urgencyNote : ""
                                color: popup.urgencyColor
                                font.pixelSize: vm.metrics.captionSize
                                font.bold: true
                            }

                            Text {
                                text: popup.age
                                color: vm.palette.textMuted
                                font.pixelSize: vm.metrics.captionSize

                                Accessible.role: Accessible.StaticText
                                Accessible.name: popup.age
                            }
                        }
                    }
                }
            }

            // Notifications held but not drawn: a sentence, not a silent gap.
            Rectangle {
                id: overflowLine

                property var line: vm.stack.overflow

                width: column.width
                height: overflowColumn.implicitHeight + 2 * vm.metrics.noticeInset
                visible: line !== null

                color: line !== null ? line.background : vm.palette.surfaceVariant
                border.color: line !== null ? line.edge : vm.palette.border
                border.width: vm.metrics.borderWidth
                radius: line !== null ? line.radius : 0

                Column {
                    id: overflowColumn

                    x: vm.metrics.noticeInset
                    y: vm.metrics.noticeInset
                    width: parent.width - 2 * vm.metrics.noticeInset

                    Text {
                        width: parent.width
                        text: overflowLine.line !== null ? overflowLine.line.text : ""
                        color: overflowLine.line !== null ? overflowLine.line.textColor : vm.palette.textMuted
                        font.pixelSize: vm.metrics.captionSize
                        wrapMode: Text.WrapAtWordBoundaryOrAnywhere

                        Accessible.role: Accessible.StaticText
                        Accessible.name: text
                    }
                }
            }

            // The daemon's own state, where the popups would be: why the stack is
            // empty, or why it is muted.
            Rectangle {
                id: noticeLine

                property var line: vm.stack.notice

                width: column.width
                height: noticeColumn.implicitHeight + 2 * vm.metrics.noticeInset
                visible: line !== null

                color: line !== null ? line.background : vm.palette.surfaceVariant
                border.color: line !== null ? line.edge : vm.palette.border
                border.width: vm.metrics.borderWidth
                radius: line !== null ? line.radius : 0

                Column {
                    id: noticeColumn

                    x: vm.metrics.noticeInset
                    y: vm.metrics.noticeInset
                    width: parent.width - 2 * vm.metrics.noticeInset

                    Text {
                        width: parent.width
                        text: noticeLine.line !== null ? noticeLine.line.text : ""
                        color: noticeLine.line !== null ? noticeLine.line.textColor : vm.palette.textMuted
                        font.pixelSize: vm.metrics.captionSize
                        wrapMode: Text.WrapAtWordBoundaryOrAnywhere

                        Accessible.role: Accessible.StaticText
                        Accessible.name: text
                    }
                }
            }
        }
    }
}
