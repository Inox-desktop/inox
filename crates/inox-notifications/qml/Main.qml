// The Inox notification popup stack.
//
// This tree only renders. Every colour, distance, radius, font size and
// user-visible sentence comes from the Rust-backed `Notifications` element below,
// which projects the daemon's state, the design tokens and the Fluent catalogs
// into one JSON model (`src/view.rs`). Do not add a colour, a pixel size or a
// piece of user-visible text here — add it to the projection instead (root
// AGENTS.md §4 and §7.3, docs/qml-migration-contract.md §4). Every read of the
// projection is qualified with the root id (`root.vm…`), including inside the
// delegates: an unqualified name resolves through the scope chain, and the one
// time it does not is the one time a surface throws a ReferenceError.
//
// The daemon's state machine, queue, policy and lock-state redaction all live in
// Rust. This surface reports three things back and nothing else:
//   * `pump()` — the frame timer that drains what the D-Bus transport delivered
//     on its own thread and refreshes the model when something changed;
//   * `dispatch(json)` — one action the user took (close / action / invoke / link);
//   * `reportReady(...)` — the load evidence the headless self-check waits for.
//
// Four things are worth knowing before changing this file:
//
//   * **The window is as tall as the stack it draws.** A card's height depends
//     on how many lines its body wraps to, which only the text layout knows, so
//     the height is bound to the column's implicit height plus the projected
//     margin. `root.vm.window.height` is the *placement* budget — what
//     `StackGeometry::window_size` reserves for a layer surface — and is not the
//     height to draw at: binding the window to it is what clipped the second and
//     third card off the bottom of the surface. The queue caps the live stack at
//     `max_visible`, so following the content cannot grow without bound.
//   * **The surface only exists when it has something to show.** `visible` is
//     bound to a projected flag: with an empty stack the daemon puts no window on
//     screen (the timer below keeps running either way — it has to, or the daemon
//     would never notice the first notification).
//   * **The body is rich text the projection wrote.** `bodyHtml` is built in
//     Rust from sanitised spans with every character escaped, and the plain
//     sentence stays in `bodyText` for the semantic tree. Rendering the body as
//     one rich-text `Text` is what makes it wrap inside the card and what draws
//     the underline the model keeps.
//   * **Entrance motion is token-timed and ends where the card rests.** The two
//     animations below write towards the resting values (opacity 1, no
//     translation), so a card whose animation never runs is fully readable
//     (ui-guidelines §8). They touch no input area and take no focus.
pragma ComponentBehavior: Bound

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

    width: root.vm.window.width
    height: column.implicitHeight + 2 * root.vm.metrics.margin
    title: root.vm.window.title
    color: root.vm.palette.background
    visible: root.vm.window.visible

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
        anchors.margins: root.vm.metrics.margin

        Accessible.role: Accessible.Pane
        Accessible.name: root.vm.window.title

        Column {
            id: column

            width: Math.min(stack.width, root.vm.metrics.width)
            spacing: root.vm.metrics.cardGap

            anchors.left: root.vm.alignment.horizontal === "left" ? parent.left : undefined
            anchors.right: root.vm.alignment.horizontal === "right" ? parent.right : undefined
            anchors.top: root.vm.alignment.vertical === "top" ? parent.top : undefined
            anchors.bottom: root.vm.alignment.vertical === "bottom" ? parent.bottom : undefined

            // One card per notification, newest first — the order is the
            // projection's, not the surface's.
            Repeater {
                model: root.vm.stack.popups

                delegate: Rectangle {
                    id: cardItem

                    // The card's data, injected by the view through a required
                    // property: the delegate is a bound component, so the model
                    // role has to be named here rather than read from a context
                    // property it would silently fall back to.
                    required property var modelData

                    // Read by the self-check: the tree really instantiated this
                    // popup, rather than merely being told about it.
                    property bool popupCard: true
                    property var popup: modelData

                    width: column.width
                    height: cardColumn.implicitHeight + 2 * root.vm.metrics.cardInset

                    color: cardItem.popup.card.background
                    border.color: cardItem.popup.card.edge
                    border.width: root.vm.metrics.borderWidth
                    radius: cardItem.popup.card.radius

                    Accessible.role: Accessible.Pane
                    Accessible.name: cardItem.popup.summary

                    // The resting state is the final state: opacity 1 and no
                    // translation. The animations below move *towards* these
                    // values, so a card is fully visible and correctly placed
                    // even if they never run (ui-guidelines §8).
                    opacity: 1

                    transform: Translate {
                        id: entryRise

                        y: 0
                    }

                    // Entrance motion: a short fade and a rise of one token step,
                    // both timed by `motion.fast` and eased out. A delegate is
                    // rebuilt when the whole projection changes, so a card that
                    // is already on screen re-enters; the duration is short
                    // enough that this reads as a refresh rather than a flicker.
                    NumberAnimation {
                        target: cardItem
                        property: "opacity"
                        from: 0
                        to: 1
                        duration: root.vm.motion.fast
                        easing.type: Easing.OutCubic
                        running: true
                    }

                    NumberAnimation {
                        target: entryRise
                        property: "y"
                        from: root.vm.motion.rise
                        to: 0
                        duration: root.vm.motion.fast
                        easing.type: Easing.OutCubic
                        running: true
                    }

                    // The urgency rail: a mark on the leading edge of a critical
                    // card, in the same token colour as its urgency sentence. It
                    // is inset past the card's own corner so it never pokes
                    // through it, and it is decoration: the sentence on the card
                    // is what says "Critical" (ui-guidelines §5, §6).
                    Rectangle {
                        visible: cardItem.popup.card.rail.visible
                        x: root.vm.metrics.borderWidth
                        y: cardItem.popup.card.rail.inset
                        width: cardItem.popup.card.rail.width
                        height: parent.height - 2 * cardItem.popup.card.rail.inset
                        radius: cardItem.popup.card.rail.radius
                        color: cardItem.popup.card.rail.color
                    }

                    Column {
                        id: cardColumn

                        x: root.vm.metrics.cardInset
                        y: root.vm.metrics.cardInset
                        width: parent.width - 2 * root.vm.metrics.cardInset
                        spacing: root.vm.metrics.sectionGap

                        // The header: the application's icon's place, who is
                        // speaking, and the close button.
                        Item {
                            width: parent.width
                            height: Math.max(headerRow.height, closeButton.height)

                            Row {
                                id: headerRow

                                anchors.left: parent.left
                                anchors.verticalCenter: parent.verticalCenter
                                spacing: root.vm.metrics.inlineGap

                                // The space an application icon occupies. This
                                // daemon does not decode icons yet, so it draws
                                // the placeholder rather than a hole.
                                Rectangle {
                                    visible: cardItem.popup.icon.present
                                    width: cardItem.popup.icon.size
                                    height: cardItem.popup.icon.size
                                    radius: cardItem.popup.icon.radius
                                    color: cardItem.popup.icon.fill
                                }

                                Text {
                                    visible: cardItem.popup.appName !== null
                                    text: cardItem.popup.appName !== null ? cardItem.popup.appName : ""
                                    color: root.vm.palette.textMuted
                                    font.pixelSize: root.vm.metrics.captionSize

                                    Accessible.role: Accessible.StaticText
                                    Accessible.name: text
                                }
                            }

                            Rectangle {
                                id: closeButton

                                property bool hovered: closeArea.containsMouse
                                property bool pressed: closeArea.pressed
                                property var control: cardItem.popup.close.control

                                anchors.right: parent.right
                                anchors.verticalCenter: parent.verticalCenter
                                width: closeLabel.implicitWidth + 2 * root.vm.metrics.controlInset
                                height: root.vm.metrics.controlHeight
                                radius: root.vm.metrics.controlRadius

                                color: pressed ? control.press.fill
                                                : hovered ? control.hover.fill
                                                          : control.rest.fill
                                border.width: root.vm.metrics.borderWidth
                                border.color: pressed ? control.press.border
                                                      : hovered ? control.hover.border
                                                                : control.rest.border

                                Text {
                                    id: closeLabel

                                    anchors.centerIn: parent
                                    text: cardItem.popup.close.label
                                    color: closeButton.pressed ? closeButton.control.press.text
                                                   : closeButton.hovered ? closeButton.control.hover.text
                                                             : closeButton.control.rest.text
                                    font.pixelSize: root.vm.metrics.bodySize
                                }

                                Accessible.role: Accessible.Button
                                Accessible.name: cardItem.popup.close.label

                                MouseArea {
                                    id: closeArea

                                    anchors.fill: parent
                                    hoverEnabled: true
                                    cursorShape: Qt.PointingHandCursor
                                    onClicked: notifications.dispatch(JSON.stringify({
                                        "type": "close",
                                        "id": cardItem.popup.id
                                    }))
                                }
                            }
                        }

                        // The card's one heading: the notification's summary.
                        Text {
                            width: parent.width
                            text: cardItem.popup.summary
                            color: root.vm.palette.text
                            font.pixelSize: root.vm.metrics.headingSize
                            font.bold: true
                            wrapMode: Text.WrapAtWordBoundaryOrAnywhere

                            Accessible.role: Accessible.Heading
                            Accessible.name: cardItem.popup.summary
                        }

                        // The body: one run of escaped rich text, so it wraps to
                        // the card's width and draws the emphasis the model kept.
                        // A link is the only thing in it that does something when
                        // clicked, and it reports the same `link` action the span
                        // projection used to.
                        Item {
                            id: bodyBox

                            width: parent.width
                            height: bodyLine.implicitHeight
                            visible: cardItem.popup.bodyText.length > 0

                            Accessible.role: Accessible.StaticText
                            Accessible.name: cardItem.popup.bodyText

                            Text {
                                id: bodyLine

                                width: parent.width
                                text: cardItem.popup.bodyHtml
                                textFormat: Text.RichText
                                color: root.vm.palette.text
                                linkColor: root.vm.palette.accent
                                font.pixelSize: root.vm.metrics.bodySize
                                wrapMode: Text.WrapAtWordBoundaryOrAnywhere

                                onLinkActivated: function (href) {
                                    notifications.dispatch(JSON.stringify({
                                        "type": "link",
                                        "id": cardItem.popup.id,
                                        "href": href
                                    }))
                                }
                            }
                        }

                        // The actions, laid out in rows of at most
                        // `actionsPerRow` so that even sixteen actions wrap
                        // instead of running off the card (ui-guidelines §7).
                        Flow {
                            width: parent.width
                            spacing: root.vm.metrics.inlineGap
                            visible: cardItem.popup.actions.length > 0

                            Repeater {
                                model: cardItem.popup.actions

                                delegate: Rectangle {
                                    id: actionButton

                                    required property var modelData

                                    property var action: modelData
                                    property bool hovered: actionArea.containsMouse
                                    property bool pressed: actionArea.pressed

                                    width: (cardColumn.width
                                            - (root.vm.metrics.actionsPerRow - 1) * root.vm.metrics.controlGap)
                                           / root.vm.metrics.actionsPerRow
                                    height: root.vm.metrics.controlHeight
                                    radius: root.vm.metrics.controlRadius

                                    color: !action.enabled ? action.control.disabled.fill
                                           : pressed ? action.control.press.fill
                                                     : hovered ? action.control.hover.fill
                                                               : action.control.rest.fill
                                    border.width: root.vm.metrics.borderWidth
                                    border.color: !action.enabled ? action.control.disabled.border
                                                  : pressed ? action.control.press.border
                                                            : hovered ? action.control.hover.border
                                                                      : action.control.rest.border

                                    Text {
                                        anchors.centerIn: parent
                                        width: parent.width - 2 * root.vm.metrics.controlInset
                                        text: actionButton.action.label
                                        color: !actionButton.action.enabled ? actionButton.action.control.disabled.text
                                               : actionButton.pressed ? actionButton.action.control.press.text
                                                                      : actionButton.hovered ? actionButton.action.control.hover.text
                                                                                             : actionButton.action.control.rest.text
                                        font.pixelSize: root.vm.metrics.bodySize
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
                                            "id": cardItem.popup.id,
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
                            visible: cardItem.popup.actions.length > 0 && cardItem.popup.actions[0].disabledReason !== null
                            text: cardItem.popup.actions.length > 0 && cardItem.popup.actions[0].disabledReason !== null
                                  ? cardItem.popup.actions[0].disabledReason : ""
                            color: root.vm.palette.textMuted
                            font.pixelSize: root.vm.metrics.captionSize
                            wrapMode: Text.WrapAtWordBoundaryOrAnywhere

                            Accessible.role: Accessible.StaticText
                            Accessible.name: text
                        }

                        // The status line: how urgent it is (as a sentence, not
                        // only as a colour) and how long ago it arrived.
                        Row {
                            width: parent.width
                            spacing: root.vm.metrics.inlineGap

                            Text {
                                visible: cardItem.popup.urgencyNote !== null
                                text: cardItem.popup.urgencyNote !== null ? cardItem.popup.urgencyNote : ""
                                color: cardItem.popup.urgencyColor
                                font.pixelSize: root.vm.metrics.captionSize
                                font.bold: true
                            }

                            Text {
                                text: cardItem.popup.age
                                color: root.vm.palette.textMuted
                                font.pixelSize: root.vm.metrics.captionSize

                                Accessible.role: Accessible.StaticText
                                Accessible.name: cardItem.popup.age
                            }
                        }
                    }
                }
            }

            // Notifications held but not drawn: a sentence, not a silent gap.
            Rectangle {
                id: overflowLine

                property var line: root.vm.stack.overflow

                width: column.width
                height: overflowColumn.implicitHeight + 2 * root.vm.metrics.noticeInset
                visible: line !== null

                color: line !== null ? line.background : root.vm.palette.surfaceVariant
                border.color: line !== null ? line.edge : root.vm.palette.border
                border.width: root.vm.metrics.borderWidth
                radius: line !== null ? line.radius : 0

                Column {
                    id: overflowColumn

                    x: root.vm.metrics.noticeInset
                    y: root.vm.metrics.noticeInset
                    width: parent.width - 2 * root.vm.metrics.noticeInset

                    Text {
                        width: parent.width
                        text: overflowLine.line !== null ? overflowLine.line.text : ""
                        color: overflowLine.line !== null ? overflowLine.line.textColor : root.vm.palette.textMuted
                        font.pixelSize: root.vm.metrics.captionSize
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

                property var line: root.vm.stack.notice

                width: column.width
                height: noticeColumn.implicitHeight + 2 * root.vm.metrics.noticeInset
                visible: line !== null

                color: line !== null ? line.background : root.vm.palette.surfaceVariant
                border.color: line !== null ? line.edge : root.vm.palette.border
                border.width: root.vm.metrics.borderWidth
                radius: line !== null ? line.radius : 0

                Column {
                    id: noticeColumn

                    x: root.vm.metrics.noticeInset
                    y: root.vm.metrics.noticeInset
                    width: parent.width - 2 * root.vm.metrics.noticeInset

                    Text {
                        width: parent.width
                        text: noticeLine.line !== null ? noticeLine.line.text : ""
                        color: noticeLine.line !== null ? noticeLine.line.textColor : root.vm.palette.textMuted
                        font.pixelSize: root.vm.metrics.captionSize
                        wrapMode: Text.WrapAtWordBoundaryOrAnywhere

                        Accessible.role: Accessible.StaticText
                        Accessible.name: text
                    }
                }
            }
        }
    }
}
