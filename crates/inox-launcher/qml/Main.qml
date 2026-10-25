// The Inox application launcher.
//
// This tree only renders and reports. Every colour, distance, font size and
// user-visible sentence comes from the Rust-backed `Launcher` element below, which
// projects the design tokens and the Fluent catalogs — do not add a literal
// colour, a pixel size or a user-visible string here; add it to `src/view.rs`
// instead (root AGENTS.md §4 and §7.3).
//
// It reports four things back, all through one JSON invokable:
//
//   * `dispatch` — the action a control performed (typing, Enter, Escape, a click);
//   * `pump` — "apply whatever the worker threads posted" (the timer below);
//   * `reportReady` — "the tree is built and this many controls are accessible".
//
// The focus contract is the launcher's whole point: the search field takes the
// caret as the window opens, so the first keystroke goes into it.
import QtQuick
import QtQuick.Window
import com.inox.launcher.qml

Window {
    id: root

    // The projection from `src/view.rs`: one document, re-read on every change.
    property var vm: JSON.parse(app.modelJson)

    width: app.windowWidth
    height: app.windowHeight
    visible: true
    title: vm.title
    color: vm.theme.styles.window.background

    // The Rust-backed state machine, its projection and its invokables.
    Launcher {
        id: app
    }

    // Applies what the worker threads posted (an index scan, launch evidence, a
    // configuration reload) and republishes the projection when anything changed.
    Timer {
        interval: 16
        running: true
        repeat: true
        onTriggered: app.pump()
    }

    Connections {
        target: app

        // The projection is the only source of truth for the field's text, so a
        // clear action from Rust reaches the control here.
        function onModelJsonChanged() {
            root.syncQuery()
        }

        // `focusSearch` on the Rust side bumps this; the caret follows it.
        function onFocusSerialChanged() {
            field.forceActiveFocus()
        }

        // Escape asks the launcher to close (it closes one level).
        function onQuitRequestedChanged() {
            if (app.quitRequested) {
                root.close()
            }
        }
    }

    Component.onCompleted: {
        root.syncQuery()
        field.forceActiveFocus()
        app.reportReady(root.title, root.accessibleItemCount(content))
    }

    // Make the control show what the state machine believes, without fighting the
    // user's cursor: only write when the two disagree.
    function syncQuery() {
        if (field.text !== vm.query) {
            field.text = vm.query
            field.cursorPosition = field.text.length
        }
    }

    function dispatchAction(action) {
        app.dispatch(JSON.stringify(action))
    }

    // Count the items that expose an accessibility role, so the headless
    // self-check can assert the semantic tree exists instead of trusting it does
    // (root AGENTS.md §7.4; Qt's QPA bridge is what exports it to AT-SPI).
    function accessibleItemCount(item) {
        if (!item || !item.children) {
            return 0
        }
        var count = 0
        if (item.Accessible && item.Accessible.role !== undefined
                && item.Accessible.role !== Accessible.NoRole) {
            count += 1
        }
        var children = item.children
        for (var i = 0; i < children.length; ++i) {
            count += root.accessibleItemCount(children[i])
        }
        return count
    }

    FocusScope {
        id: content
        anchors.fill: parent
        anchors.margins: vm.theme.metrics.window_padding

        Accessible.role: Accessible.Pane
        Accessible.name: vm.title

        // ---- the search field: the one thing the surface is about ----------
        Rectangle {
            id: fieldBox
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.top: parent.top
            height: vm.theme.metrics.field_height
            radius: vm.theme.styles.field.radius
            color: field.activeFocus ? vm.theme.styles.field.background : vm.theme.styles.field.background_hover
            border.width: field.activeFocus
                    ? vm.theme.styles.field.focus_width
                    : vm.theme.styles.field.border_width
            border.color: field.activeFocus
                    ? vm.theme.styles.field.border_focused
                    : vm.theme.styles.field.border

            TextInput {
                id: field
                anchors.fill: parent
                anchors.leftMargin: vm.theme.metrics.control_padding_horizontal
                anchors.rightMargin: vm.theme.metrics.control_padding_horizontal
                anchors.topMargin: vm.theme.metrics.control_padding_vertical
                anchors.bottomMargin: vm.theme.metrics.control_padding_vertical
                verticalAlignment: TextInput.AlignVCenter
                clip: true
                color: vm.theme.styles.field.text
                font.family: vm.theme.font.family
                font.pixelSize: vm.theme.font.title
                selectionColor: vm.theme.styles.field.selection
                selectedTextColor: vm.theme.styles.field.selected_text

                onTextEdited: root.dispatchAction({ type: "set-query", value: field.text })
                onAccepted: root.dispatchAction({ type: "press", key: "enter" })

                // The four keys the launcher binds (src/app.rs `key_binding`).
                // Everything else is text: the field is a keyboard surface.
                Keys.onUpPressed: root.dispatchAction({ type: "press", key: "up" })
                Keys.onDownPressed: root.dispatchAction({ type: "press", key: "down" })
                Keys.onEscapePressed: root.dispatchAction({ type: "press", key: "escape" })

                Accessible.role: Accessible.EditableText
                Accessible.name: vm.placeholder
                Accessible.description: vm.hint
            }

            // The placeholder, shown only while the field is empty, so the surface
            // never needs a hard-coded string.
            Text {
                anchors.fill: field
                verticalAlignment: Text.AlignVCenter
                visible: field.text.length === 0
                text: vm.placeholder
                color: vm.theme.styles.field.placeholder
                font.family: vm.theme.font.family
                font.pixelSize: vm.theme.font.title
                elide: Text.ElideRight
            }
        }

        // ---- the result card: the heading, the rows, or the state ----------
        Rectangle {
            id: listCard
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.top: fieldBox.bottom
            anchors.topMargin: vm.theme.metrics.group_spacing
            anchors.bottom: statusBar.top
            anchors.bottomMargin: vm.theme.metrics.group_spacing
            radius: vm.theme.styles.card.radius
            color: vm.theme.styles.card.background
            border.width: vm.theme.styles.card.border_width
            border.color: vm.theme.styles.card.border

            Text {
                id: heading
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.top: parent.top
                anchors.margins: vm.theme.metrics.card_padding
                text: vm.heading
                color: vm.theme.styles.card.text
                font.family: vm.theme.font.family
                font.pixelSize: vm.theme.font.heading
                elide: Text.ElideRight

                Accessible.role: Accessible.Heading
                Accessible.name: vm.heading
            }

            // What the list says while the query is empty: the resting state, which
            // is deliberately a different screen from "no matches".
            Text {
                id: prompt
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.top: heading.bottom
                anchors.leftMargin: vm.theme.metrics.card_padding
                anchors.rightMargin: vm.theme.metrics.card_padding
                anchors.topMargin: vm.theme.metrics.space_xs
                height: visible ? implicitHeight : 0
                visible: vm.prompt !== null
                text: vm.prompt === null ? "" : vm.prompt
                color: vm.theme.colors.text_muted
                font.family: vm.theme.font.family
                font.pixelSize: vm.theme.font.caption
                wrapMode: Text.WordWrap
            }

            // The rows.
            ListView {
                id: resultList
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.top: prompt.bottom
                anchors.bottom: parent.bottom
                anchors.leftMargin: vm.theme.metrics.card_padding
                anchors.rightMargin: vm.theme.metrics.card_padding
                anchors.topMargin: vm.theme.metrics.heading_gap
                anchors.bottomMargin: vm.theme.metrics.card_padding
                visible: vm.banner === null
                clip: true
                spacing: vm.theme.metrics.list_spacing
                model: vm.rows

                delegate: Rectangle {
                    id: rowItem
                    required property var modelData
                    required property int index

                    width: ListView.view.width
                    height: vm.theme.metrics.row_height
                    radius: modelData.style.radius
                    color: rowPress.pressed
                           ? modelData.style.background_pressed
                           : (rowPress.containsMouse
                              ? modelData.style.background_hover
                              : modelData.style.background)

                    MouseArea {
                        id: rowPress
                        anchors.fill: parent
                        hoverEnabled: true
                        enabled: modelData.enabled
                        onClicked: root.dispatchAction({ type: "choose", id: modelData.id })
                    }

                    Row {
                        anchors.fill: parent
                        anchors.topMargin: vm.theme.metrics.control_padding_vertical
                        anchors.bottomMargin: vm.theme.metrics.control_padding_vertical
                        anchors.leftMargin: vm.theme.metrics.control_padding_horizontal
                        anchors.rightMargin: vm.theme.metrics.control_padding_horizontal
                        spacing: vm.theme.metrics.space_sm

                        // The icon column: a square tile, never the whole row.
                        Rectangle {
                            width: vm.theme.metrics.icon_size
                            height: vm.theme.metrics.icon_size
                            anchors.verticalCenter: parent.verticalCenter
                            radius: vm.theme.metrics.radius_sm
                            color: modelData.style.badge_background

                            Text {
                                anchors.centerIn: parent
                                text: modelData.icon
                                color: modelData.style.badge_color
                                font.family: vm.theme.font.family
                                font.pixelSize: vm.theme.font.caption
                            }
                        }

                        Column {
                            width: parent.width - vm.theme.metrics.icon_size - parent.spacing
                            anchors.verticalCenter: parent.verticalCenter
                            spacing: vm.theme.metrics.space_xs

                            Text {
                                width: parent.width
                                text: modelData.label
                                color: modelData.style.text
                                font.family: vm.theme.font.family
                                font.pixelSize: vm.theme.font.body
                                elide: Text.ElideRight
                            }

                            Text {
                                width: parent.width
                                visible: modelData.detail !== null
                                text: modelData.detail === null ? "" : modelData.detail
                                color: modelData.style.detail
                                font.family: vm.theme.font.family
                                font.pixelSize: vm.theme.font.caption
                                elide: Text.ElideRight
                            }

                            Text {
                                width: parent.width
                                visible: modelData.disabled_reason !== null
                                text: modelData.disabled_reason === null ? "" : modelData.disabled_reason
                                color: modelData.style.disabled_reason
                                font.family: vm.theme.font.family
                                font.pixelSize: vm.theme.font.caption
                                wrapMode: Text.WordWrap
                            }
                        }
                    }

                    Accessible.role: Accessible.Button
                    Accessible.name: modelData.label
                    // Qt's `Accessible` attached type has no "unavailable" state
                    // (Qt 6.12), so a row that cannot be launched says why in the
                    // description instead; the reason is already a Fluent string
                    // from the projection.
                    Accessible.description: modelData.disabled_reason !== null
                            ? modelData.disabled_reason
                            : (modelData.detail === null ? "" : modelData.detail)
                    Accessible.focusable: false
                }
            }

            // The state that stands in for the rows: loading, empty, no match, or
            // an index that could not be read (acceptance D05).
            Column {
                id: banner
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.top: prompt.bottom
                anchors.leftMargin: vm.theme.metrics.card_padding
                anchors.rightMargin: vm.theme.metrics.card_padding
                anchors.topMargin: vm.theme.metrics.heading_gap
                visible: vm.banner !== null
                spacing: vm.theme.metrics.list_spacing

                Text {
                    width: parent.width
                    text: vm.banner === null ? "" : vm.banner.title
                    color: vm.theme.styles.card.text
                    font.family: vm.theme.font.family
                    font.pixelSize: vm.theme.font.heading
                    wrapMode: Text.WordWrap

                    Accessible.role: Accessible.StaticText
                    Accessible.name: text
                }

                Text {
                    width: parent.width
                    visible: text.length > 0
                    text: vm.banner === null || vm.banner.body === null ? "" : vm.banner.body
                    color: vm.theme.colors.text_muted
                    font.family: vm.theme.font.family
                    font.pixelSize: vm.theme.font.caption
                    wrapMode: Text.WordWrap
                }

                Text {
                    width: parent.width
                    visible: text.length > 0
                    text: vm.banner === null || vm.banner.hint === null ? "" : vm.banner.hint
                    color: vm.theme.colors.text_muted
                    font.family: vm.theme.font.family
                    font.pixelSize: vm.theme.font.caption
                    wrapMode: Text.WordWrap
                }

                // The one action a state offers, and only the one it can carry out.
                Rectangle {
                    id: bannerAction
                    width: bannerActionLabel.implicitWidth
                            + vm.theme.metrics.control_padding_horizontal * 2
                    height: bannerActionLabel.implicitHeight
                            + vm.theme.metrics.control_padding_vertical * 2
                    visible: vm.banner !== null && vm.banner.action !== null
                    radius: vm.theme.styles.action.radius
                    color: bannerActionPress.pressed
                           ? vm.theme.styles.action.background_pressed
                           : (bannerActionPress.containsMouse
                              ? vm.theme.styles.action.background_hover
                              : vm.theme.styles.action.background)
                    border.width: vm.theme.styles.action.border_width
                    border.color: vm.theme.styles.action.border

                    Text {
                        id: bannerActionLabel
                        anchors.centerIn: parent
                        text: vm.banner === null || vm.banner.action === null ? "" : vm.banner.action.label
                        color: bannerActionPress.pressed
                               ? vm.theme.styles.action.text_pressed
                               : vm.theme.styles.action.text
                        font.family: vm.theme.font.family
                        font.pixelSize: vm.theme.font.body
                    }

                    MouseArea {
                        id: bannerActionPress
                        anchors.fill: parent
                        hoverEnabled: true
                        onClicked: root.dispatchAction({ type: vm.banner.action.type })
                    }

                    Accessible.role: Accessible.Button
                    Accessible.name: bannerActionLabel.text
                }
            }
        }

        // ---- the status bar: the key hints, notices, the launch ------------
        Rectangle {
            id: statusBar
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            height: statusColumn.height + vm.theme.metrics.card_padding * 2
            radius: vm.theme.styles.card.radius
            color: vm.theme.styles.card.background
            border.width: vm.theme.styles.card.border_width
            border.color: vm.theme.styles.card.border

            Column {
                id: statusColumn
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.top: parent.top
                anchors.margins: vm.theme.metrics.card_padding
                spacing: vm.theme.metrics.list_spacing

                Text {
                    width: parent.width
                    text: vm.hint
                    color: vm.theme.colors.text_muted
                    font.family: vm.theme.font.family
                    font.pixelSize: vm.theme.font.caption

                    Accessible.role: Accessible.StaticText
                    Accessible.name: vm.hint
                }

                Text {
                    width: parent.width
                    visible: text.length > 0
                    text: vm.notice === null ? "" : vm.notice
                    color: vm.theme.colors.text_muted
                    font.family: vm.theme.font.family
                    font.pixelSize: vm.theme.font.caption
                    wrapMode: Text.WordWrap
                }

                Text {
                    width: parent.width
                    visible: vm.status !== null
                    text: vm.status === null ? "" : vm.status.line
                    color: vm.status === null ? vm.theme.colors.text : vm.status.color
                    font.family: vm.theme.font.family
                    font.pixelSize: vm.theme.font.body
                    wrapMode: Text.WordWrap
                }

                Text {
                    width: parent.width
                    visible: text.length > 0
                    text: vm.status === null || vm.status.reason === null ? "" : vm.status.reason
                    color: vm.theme.colors.text_muted
                    font.family: vm.theme.font.family
                    font.pixelSize: vm.theme.font.caption
                    wrapMode: Text.WordWrap
                }

                // Retrying a launch is the only action the status bar offers.
                Rectangle {
                    id: retryAction
                    width: retryActionLabel.implicitWidth
                            + vm.theme.metrics.control_padding_horizontal * 2
                    height: retryActionLabel.implicitHeight
                            + vm.theme.metrics.control_padding_vertical * 2
                    visible: vm.status !== null && vm.status.retry !== null
                    radius: vm.theme.styles.action.radius
                    color: retryPress.pressed
                           ? vm.theme.styles.action.background_pressed
                           : (retryPress.containsMouse
                              ? vm.theme.styles.action.background_hover
                              : vm.theme.styles.action.background)
                    border.width: vm.theme.styles.action.border_width
                    border.color: vm.theme.styles.action.border

                    Text {
                        id: retryActionLabel
                        anchors.centerIn: parent
                        text: vm.status === null || vm.status.retry === null ? "" : vm.status.retry.label
                        color: retryPress.pressed
                               ? vm.theme.styles.action.text_pressed
                               : vm.theme.styles.action.text
                        font.family: vm.theme.font.family
                        font.pixelSize: vm.theme.font.body
                    }

                    MouseArea {
                        id: retryPress
                        anchors.fill: parent
                        hoverEnabled: true
                        onClicked: root.dispatchAction({ type: vm.status.retry.type })
                    }

                    Accessible.role: Accessible.Button
                    Accessible.name: retryActionLabel.text
                }
            }
        }
    }
}
