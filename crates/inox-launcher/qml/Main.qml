// The Inox application launcher.
//
// A grid of applications, the way a person looks for one: the surface title and
// the search field share the top row, the category chips under them say what the
// grid is showing, the card holds the tiles, and the hint strip at the bottom says
// which keys are bound.
//
// This tree only renders and reports. Every colour, distance, font size, icon
// file and user-visible sentence comes from the Rust-backed `Launcher` element
// below, which projects the design tokens, the resolved icons and the Fluent
// catalogs — do not add a literal colour, a pixel size or a user-visible string
// here; add it to `src/view.rs` instead (root AGENTS.md §4 and §7.3).
//
// Every projection read is written as `root.vm.…`, including inside the tile and
// chip delegates. The scope chain would let an unqualified `vm` resolve through a
// parent element, which works by accident and is exactly the class of bug
// `qmllint`'s `unqualified` diagnostic exists to catch.
//
// It reports three things back, all through the bridge's invokables:
//
//   * `dispatch` — the action a control performed (typing, an arrow, Enter,
//     Escape, a click on a tile or a chip);
//   * `pump` — "apply whatever the worker threads posted" (the timer below);
//   * `reportReady` — "the tree is built and this many controls are accessible".
//
// The focus contract is the launcher's whole point: the search field takes the
// caret as the window opens, so the first keystroke goes into it.
//
// The delegates below resolve `root` and the shared metrics through this file's
// scope on purpose; without the pragma their bindings are compiled against the
// delegate's own context and `qmllint` flags every such read.
pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Window
import com.inox.launcher.qml

Window {
    id: root

    // The projection from `src/view.rs`: one document, re-read on every change.
    property var vm: JSON.parse(app.modelJson)

    // Where the projection put the selection, or -1 for "no tile is selected".
    // The grid only scrolls to this number; which tile is the selection is the
    // state machine's decision, not this file's.
    readonly property int selectedIndex: root.vm.selected === null || root.vm.selected === undefined
                                        ? -1
                                        : root.vm.selected

    // The height of one grid row: the tile plus the gap after it. The grid lays
    // tiles out on this pitch and draws whole rows only (see the delegate's
    // `height`), so the last visible row is never half a tile.
    readonly property real rowPitch: root.vm.grid.cell_height + root.vm.grid.gap

    width: app.windowWidth
    height: app.windowHeight
    visible: true
    title: root.vm.title
    color: root.vm.theme.styles.window.background

    // The window fades in over the token's `fast` duration as it opens. Opacity's
    // resting value is the Window's own (fully visible), so the surface is never
    // visible *only* through the motion: with animations switched off the user
    // lands straight on the end state (guidelines §8). Nothing else fades — typing
    // and scrolling stay instant.
    NumberAnimation {
        target: root
        property: "opacity"
        from: 0
        to: 1
        duration: root.vm.theme.motion.fast
        easing.type: Easing.OutCubic
        running: true
    }

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
            // The tiles may have been replaced and the selection may have moved;
            // let the delegate layout settle for a frame before scrolling to it.
            Qt.callLater(root.followSelection)
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
        // One frame later, so the grid has created the tiles it is asked to
        // scroll to; at this point the view has only the model's shape.
        Qt.callLater(root.followSelection)
        app.reportReady(root.title, root.accessibleItemCount(content))
    }

    // Make the control show what the state machine believes, without fighting the
    // user's cursor: only write when the two disagree.
    function syncQuery() {
        if (field.text !== root.vm.query) {
            field.text = root.vm.query
            field.cursorPosition = field.text.length
        }
    }

    // Keep the selected tile on screen.
    //
    // `GridView.Contain` scrolls only as far as it has to — a tile that is already
    // wholly visible is left where it is — so this is safe to run on every
    // projection change rather than tracking a "did the selection move" flag here.
    // The rows it crosses are the rows `root.vm.grid.columns` describes, which is
    // the number the arrow keys move by in Rust.
    function followSelection() {
        if (root.selectedIndex < 0) {
            return
        }
        grid.positionViewAtIndex(root.selectedIndex, GridView.Contain)
        root.clampGrid()
    }

    // Keep the grid's scroll position inside its content.
    //
    // While a grid creates its delegates it estimates where a tile is, and
    // `positionViewAtIndex` can then come to rest past the end of the content —
    // which draws a selected tile nowhere. The selection is what has to stay
    // visible, so the view is never allowed to settle outside its own bounds.
    function clampGrid() {
        var limit = Math.max(0, grid.contentHeight - grid.height)
        if (grid.contentY > limit) {
            grid.contentY = limit
        } else if (grid.contentY < 0) {
            grid.contentY = 0
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
        anchors.margins: root.vm.theme.metrics.window_padding

        Accessible.role: Accessible.Pane
        Accessible.name: root.vm.title

        // ---- the top row: the surface title and the search field -----------
        Item {
            id: topRow
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.top: parent.top
            height: root.vm.theme.metrics.field_height

            Text {
                id: heading
                anchors.left: parent.left
                anchors.verticalCenter: parent.verticalCenter
                text: root.vm.heading
                color: root.vm.theme.colors.text
                font.family: root.vm.theme.font.family
                font.pixelSize: root.vm.theme.font.heading
                elide: Text.ElideRight

                Accessible.role: Accessible.Heading
                Accessible.name: root.vm.heading
            }

            // The one thing the surface is about. It keeps the `title` role, so it
            // stays the heaviest element on the screen (guidelines §2).
            Rectangle {
                id: fieldBox
                anchors.left: heading.right
                anchors.leftMargin: root.vm.theme.metrics.space_lg
                anchors.right: parent.right
                anchors.verticalCenter: parent.verticalCenter
                height: root.vm.theme.metrics.field_height
                radius: root.vm.theme.styles.field.radius
                color: field.activeFocus
                       ? root.vm.theme.styles.field.background
                       : root.vm.theme.styles.field.background_hover
                border.width: field.activeFocus
                        ? root.vm.theme.styles.field.focus_width
                        : root.vm.theme.styles.field.border_width
                border.color: field.activeFocus
                        ? root.vm.theme.styles.field.border_focused
                        : root.vm.theme.styles.field.border

                TextInput {
                    id: field
                    anchors.fill: parent
                    anchors.leftMargin: root.vm.theme.metrics.control_padding_horizontal
                    anchors.rightMargin: root.vm.theme.metrics.control_padding_horizontal
                    anchors.topMargin: root.vm.theme.metrics.control_padding_vertical
                    anchors.bottomMargin: root.vm.theme.metrics.control_padding_vertical
                    verticalAlignment: TextInput.AlignVCenter
                    clip: true
                    color: root.vm.theme.styles.field.text
                    font.family: root.vm.theme.font.family
                    font.pixelSize: root.vm.theme.font.title
                    selectionColor: root.vm.theme.styles.field.selection
                    selectedTextColor: root.vm.theme.styles.field.selected_text

                    onTextEdited: root.dispatchAction({ type: "set-query", value: field.text })
                    onAccepted: root.dispatchAction({ type: "press", key: "enter" })

                    // The keys the launcher binds (`src/app.rs` `key_binding`): the
                    // arrows walk the grid, Enter launches, Escape closes. Left and
                    // Right move the selection rather than the text caret, so the
                    // launcher is driven with the arrows alone.
                    Keys.onUpPressed: root.dispatchAction({ type: "press", key: "up" })
                    Keys.onDownPressed: root.dispatchAction({ type: "press", key: "down" })
                    Keys.onLeftPressed: root.dispatchAction({ type: "press", key: "left" })
                    Keys.onRightPressed: root.dispatchAction({ type: "press", key: "right" })
                    Keys.onEscapePressed: root.dispatchAction({ type: "press", key: "escape" })

                    Accessible.role: Accessible.EditableText
                    Accessible.name: root.vm.placeholder
                    Accessible.description: root.vm.hint
                }

                // The placeholder, shown only while the field is empty, so the
                // surface never needs a hard-coded string.
                Text {
                    anchors.fill: field
                    verticalAlignment: Text.AlignVCenter
                    visible: field.text.length === 0
                    text: root.vm.placeholder
                    color: root.vm.theme.styles.field.placeholder
                    font.family: root.vm.theme.font.family
                    font.pixelSize: root.vm.theme.font.title
                    elide: Text.ElideRight
                }
            }
        }

        // ---- the category chips: which slice of the grid is showing --------
        Flow {
            id: chipRow
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.top: topRow.bottom
            anchors.topMargin: root.vm.theme.metrics.space_md
            height: implicitHeight
            // The smallest gap: the stock category set is one row at the surface
            // width the placement asks for, and this is the measurement that
            // makes it so (`space_sm` splits it in two on a 1024 px surface).
            spacing: root.vm.theme.metrics.space_xs

            Repeater {
                model: root.vm.chips

                delegate: Rectangle {
                    id: chip
                    required property var modelData

                    width: chipLabel.implicitWidth
                            + root.vm.theme.metrics.control_padding_horizontal * 2
                    height: root.vm.theme.metrics.action_height
                    radius: chip.modelData.style.radius
                    color: chipPress.pressed
                           ? chip.modelData.style.background_pressed
                           : (chipPress.containsMouse
                              ? chip.modelData.style.background_hover
                              : chip.modelData.style.background)
                    border.width: chip.modelData.style.border_width
                    border.color: chip.modelData.style.border

                    Text {
                        id: chipLabel
                        anchors.centerIn: parent
                        text: chip.modelData.label
                        color: chipPress.pressed
                               ? chip.modelData.style.text_pressed
                               : chip.modelData.style.text
                        font.family: root.vm.theme.font.family
                        font.pixelSize: root.vm.theme.font.body
                    }

                    MouseArea {
                        id: chipPress
                        anchors.fill: parent
                        hoverEnabled: true
                        cursorShape: Qt.PointingHandCursor
                        onClicked: root.dispatchAction({
                            type: "select-category",
                            category: chip.modelData.id,
                        })
                    }

                    Accessible.role: Accessible.Button
                    Accessible.name: chipLabel.text
                }
            }
        }

        // ---- the grid card: the tiles, or the state that replaces them -----
        Rectangle {
            id: listCard
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.top: chipRow.bottom
            anchors.topMargin: root.vm.theme.metrics.space_md
            anchors.bottom: statusBar.top
            anchors.bottomMargin: root.vm.theme.metrics.group_spacing
            radius: root.vm.theme.styles.card.radius
            color: root.vm.theme.styles.card.background
            border.width: root.vm.theme.styles.card.border_width
            border.color: root.vm.theme.styles.card.border

            Item {
                id: cardContent
                anchors.fill: parent
                anchors.margins: root.vm.theme.metrics.card_padding

                // The applications. The cell size and the column count come from
                // the projection, so the tiles the window draws are the tiles the
                // keyboard steps by; a grid taller than the card scrolls.
                //
                // The card is the grid: there is no line above it. The search
                // placeholder says what the field is for, and a browsing sentence
                // would be a second answer to a question the field already asks.
                GridView {
                    id: grid
                    // Exactly the cells the projection names, centred in the card:
                    // the columns the keyboard steps by are the columns drawn, and
                    // the card's slack sits on both sides instead of only the right.
                    anchors.horizontalCenter: parent.horizontalCenter
                    anchors.top: parent.top
                    width: root.vm.grid.columns
                           * (root.vm.grid.cell_width + root.vm.grid.gap)
                    // Whole rows only. The card's height is what the hint strip
                    // leaves, which changes when a notice or a launch status
                    // appears; the grid draws whole tiles and leaves the slack
                    // below the last row rather than cutting one in half.
                    height: Math.min(parent.height,
                                     Math.max(root.rowPitch,
                                              Math.floor(parent.height / root.rowPitch) * root.rowPitch))
                    // Cross-fade with the banner: the tokens' `normal` duration on
                    // opacity, and a resting opacity of 1 (shown) or 0 (replaced).
                    // Visibility is never *only* the animation's end state.
                    visible: opacity > 0
                    opacity: root.vm.banner === null ? 1 : 0
                    clip: true
                    cellWidth: root.vm.grid.cell_width + root.vm.grid.gap
                    cellHeight: root.rowPitch
                    model: root.vm.tiles

                    // The content grows as the delegates are created; a scroll
                    // position computed against an estimate is corrected here
                    // rather than left past the end.
                    onContentHeightChanged: root.clampGrid()
                    onContentYChanged: root.clampGrid()

                    Behavior on opacity {
                        NumberAnimation {
                            duration: root.vm.theme.motion.normal
                            easing.type: Easing.OutCubic
                        }
                    }

                    delegate: Item {
                        id: tile
                        required property var modelData
                        required property int index

                        width: root.vm.grid.cell_width
                        height: root.vm.grid.cell_height

                        Rectangle {
                            id: tileBackground
                            anchors.fill: parent
                            radius: root.vm.theme.metrics.radius_md
                            color: tilePress.pressed
                                   ? tile.modelData.style.background_pressed
                                   : (tilePress.containsMouse
                                      ? tile.modelData.style.background_hover
                                      : tile.modelData.style.background)

                            // Hover and press are pointer-driven state changes; the
                            // token's `fast` duration keeps them from snapping
                            // without adding motion to anything the keyboard drives
                            // (§8).
                            Behavior on color {
                                ColorAnimation {
                                    duration: root.vm.theme.motion.fast
                                    easing.type: Easing.OutCubic
                                }
                            }
                        }

                        Column {
                            anchors.left: parent.left
                            anchors.right: parent.right
                            anchors.verticalCenter: parent.verticalCenter
                            spacing: root.vm.theme.metrics.space_xs

                            // The icon: the file the index resolved, or the letter
                            // the entry always had. The letter is also what a file
                            // Qt cannot decode falls back to, so a broken icon is
                            // visible as a missing one rather than as a hole.
                            Item {
                                width: parent.width
                                height: root.vm.grid.image

                                Image {
                                    id: appIcon
                                    anchors.centerIn: parent
                                    width: root.vm.grid.image
                                    height: root.vm.grid.image
                                    sourceSize.width: root.vm.grid.image
                                    sourceSize.height: root.vm.grid.image
                                    fillMode: Image.PreserveAspectFit
                                    smooth: true
                                    source: tile.modelData.icon_url === null
                                            ? ""
                                            : tile.modelData.icon_url
                                    visible: tile.modelData.icon_url !== null
                                             && status === Image.Ready
                                }

                                Rectangle {
                                    id: letterTile
                                    anchors.centerIn: parent
                                    width: root.vm.grid.image
                                    height: root.vm.grid.image
                                    visible: !appIcon.visible
                                    radius: root.vm.theme.metrics.radius_sm
                                    color: tile.modelData.style.badge_background

                                    Text {
                                        anchors.centerIn: parent
                                        text: tile.modelData.icon
                                        color: tile.modelData.style.badge_color
                                        font.family: root.vm.theme.font.family
                                        font.pixelSize: root.vm.theme.font.title
                                    }
                                }
                            }

                            Text {
                                id: tileLabel
                                width: parent.width
                                text: tile.modelData.label
                                color: tile.modelData.style.text
                                font.family: root.vm.theme.font.family
                                font.pixelSize: root.vm.theme.font.caption
                                horizontalAlignment: Text.AlignHCenter
                                wrapMode: Text.Wrap
                                maximumLineCount: root.vm.grid.label_lines
                                elide: Text.ElideRight
                            }
                        }

                        MouseArea {
                            id: tilePress
                            anchors.fill: parent
                            hoverEnabled: true
                            enabled: tile.modelData.enabled
                            // The hand only appears where a click would do
                            // something: an unusable tile may be selected (so Enter
                            // can explain why), but it is not a target.
                            cursorShape: tile.modelData.enabled
                                         ? Qt.PointingHandCursor
                                         : Qt.ArrowCursor
                            onClicked: root.dispatchAction({
                                type: "choose",
                                id: tile.modelData.id,
                            })
                        }

                        Accessible.role: Accessible.Button
                        Accessible.name: tile.modelData.label
                        // Qt's `Accessible` attached type has no "unavailable" state
                        // (Qt 6.12), so a tile that cannot be launched says why in
                        // the description instead; the reason is already a Fluent
                        // string from the projection.
                        Accessible.description: tile.modelData.disabled_reason !== null
                                ? tile.modelData.disabled_reason
                                : (tile.modelData.detail === null ? "" : tile.modelData.detail)
                        Accessible.focusable: false
                    }
                }

                // A grid taller than the card has to say so. Only the thumb is
                // drawn: a track running the grid's full height at the card's edge
                // reads as a border that belongs to the card. The thumb is a
                // rounded pill one `space_xs` wide in `text_muted`, a third of the
                // grid's height, inset from the grid's edge, present only while the
                // tiles overflow, and it follows the grid's own `contentY` — so the
                // affordance is drawn from tokens and the scroll itself is not
                // animated (guidelines §8).
                Rectangle {
                    id: scrollThumb
                    width: root.vm.theme.metrics.space_xs
                    height: grid.height / 3
                    radius: width / 2
                    color: root.vm.theme.colors.text_muted
                    visible: root.vm.banner === null
                             && grid.contentHeight > grid.height
                    x: grid.x + grid.width - width - root.vm.theme.metrics.space_xs
                    y: {
                        var offset = 0
                        var overflow = grid.contentHeight - grid.height
                        if (overflow > 0) {
                            var travel = grid.height - scrollThumb.height
                            offset = Math.max(0, Math.min(travel, grid.contentY / overflow * travel))
                        }
                        return grid.y + offset
                    }
                }

                // The state that stands in for the grid: loading, empty, no match,
                // or an index that could not be read (acceptance D05). It is
                // centred in the card: a short sentence floating at the top of a
                // tall empty card reads as a grid that failed to draw, not as an
                // answered screen.
                Column {
                    id: banner
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.verticalCenter: parent.verticalCenter
                    spacing: root.vm.theme.metrics.list_spacing
                    // Same cross-fade as the grid, in the other direction.
                    visible: opacity > 0
                    opacity: root.vm.banner === null ? 0 : 1

                    Behavior on opacity {
                        NumberAnimation {
                            duration: root.vm.theme.motion.normal
                            easing.type: Easing.OutCubic
                        }
                    }

                    Text {
                        width: parent.width
                        text: root.vm.banner === null ? "" : root.vm.banner.title
                        color: root.vm.theme.styles.card.text
                        font.family: root.vm.theme.font.family
                        font.pixelSize: root.vm.theme.font.heading
                        wrapMode: Text.WordWrap

                        Accessible.role: Accessible.StaticText
                        Accessible.name: text
                    }

                    Text {
                        width: parent.width
                        visible: text.length > 0
                        text: root.vm.banner === null || root.vm.banner.body === null
                              ? "" : root.vm.banner.body
                        color: root.vm.theme.colors.text_muted
                        font.family: root.vm.theme.font.family
                        font.pixelSize: root.vm.theme.font.caption
                        wrapMode: Text.WordWrap
                    }

                    Text {
                        width: parent.width
                        visible: text.length > 0
                        text: root.vm.banner === null || root.vm.banner.hint === null
                              ? "" : root.vm.banner.hint
                        color: root.vm.theme.colors.text_muted
                        font.family: root.vm.theme.font.family
                        font.pixelSize: root.vm.theme.font.caption
                        wrapMode: Text.WordWrap
                    }

                    // The one action a state offers, and only the one it can carry
                    // out. Its height is the projected `action_height` — one line
                    // plus the control padding, never below `spacing.xl` — rather
                    // than a label plus padding added up in this file.
                    Rectangle {
                        id: bannerAction
                        width: bannerActionLabel.implicitWidth
                                + root.vm.theme.metrics.control_padding_horizontal * 2
                        height: root.vm.theme.metrics.action_height
                        visible: root.vm.banner !== null && root.vm.banner.action !== null
                        radius: root.vm.theme.styles.action.radius
                        color: bannerActionPress.pressed
                               ? root.vm.theme.styles.action.background_pressed
                               : (bannerActionPress.containsMouse
                                  ? root.vm.theme.styles.action.background_hover
                                  : root.vm.theme.styles.action.background)
                        border.width: root.vm.theme.styles.action.border_width
                        border.color: root.vm.theme.styles.action.border

                        Text {
                            id: bannerActionLabel
                            anchors.centerIn: parent
                            text: root.vm.banner === null || root.vm.banner.action === null
                                  ? "" : root.vm.banner.action.label
                            color: bannerActionPress.pressed
                                   ? root.vm.theme.styles.action.text_pressed
                                   : root.vm.theme.styles.action.text
                            font.family: root.vm.theme.font.family
                            font.pixelSize: root.vm.theme.font.body
                        }

                        MouseArea {
                            id: bannerActionPress
                            anchors.fill: parent
                            hoverEnabled: true
                            cursorShape: Qt.PointingHandCursor
                            onClicked: root.dispatchAction({ type: root.vm.banner.action.type })
                        }

                        Accessible.role: Accessible.Button
                        Accessible.name: bannerActionLabel.text
                    }
                }
            }
        }

        // ---- the hint strip: the key hints, notices, the launch ------------
        Rectangle {
            id: statusBar
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            height: statusColumn.height + root.vm.theme.metrics.card_padding * 2
            radius: root.vm.theme.styles.card.radius
            color: root.vm.theme.styles.card.background
            border.width: root.vm.theme.styles.card.border_width
            border.color: root.vm.theme.styles.card.border

            Column {
                id: statusColumn
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.top: parent.top
                anchors.margins: root.vm.theme.metrics.card_padding
                spacing: root.vm.theme.metrics.list_spacing

                // The projection decides which keys this screen binds: with tiles
                // it names the arrows and Enter, without them it is the close hint
                // alone.
                Text {
                    width: parent.width
                    height: root.vm.theme.metrics.caption_line_height
                    text: root.vm.hint
                    color: root.vm.theme.colors.text_muted
                    font.family: root.vm.theme.font.family
                    font.pixelSize: root.vm.theme.font.caption

                    Accessible.role: Accessible.StaticText
                    Accessible.name: root.vm.hint
                }

                // One caption line, elided: the placement budgets exactly this
                // line for the skipped-entries notice, so the grid below it keeps
                // its whole rows.
                Text {
                    width: parent.width
                    height: root.vm.theme.metrics.caption_line_height
                    visible: text.length > 0
                    text: root.vm.notice === null ? "" : root.vm.notice
                    color: root.vm.theme.colors.text_muted
                    font.family: root.vm.theme.font.family
                    font.pixelSize: root.vm.theme.font.caption
                    elide: Text.ElideRight
                }

                Text {
                    width: parent.width
                    visible: root.vm.status !== null
                    text: root.vm.status === null ? "" : root.vm.status.line
                    color: root.vm.status === null ? root.vm.theme.colors.text : root.vm.status.color
                    font.family: root.vm.theme.font.family
                    font.pixelSize: root.vm.theme.font.body
                    wrapMode: Text.WordWrap
                }

                Text {
                    width: parent.width
                    visible: text.length > 0
                    text: root.vm.status === null || root.vm.status.reason === null
                          ? "" : root.vm.status.reason
                    color: root.vm.theme.colors.text_muted
                    font.family: root.vm.theme.font.family
                    font.pixelSize: root.vm.theme.font.caption
                    wrapMode: Text.WordWrap
                }

                // Retrying a launch is the only action the status bar offers; it is
                // the same projected height as the banner's action.
                Rectangle {
                    id: retryAction
                    width: retryActionLabel.implicitWidth
                            + root.vm.theme.metrics.control_padding_horizontal * 2
                    height: root.vm.theme.metrics.action_height
                    visible: root.vm.status !== null && root.vm.status.retry !== null
                    radius: root.vm.theme.styles.action.radius
                    color: retryPress.pressed
                           ? root.vm.theme.styles.action.background_pressed
                           : (retryPress.containsMouse
                              ? root.vm.theme.styles.action.background_hover
                              : root.vm.theme.styles.action.background)
                    border.width: root.vm.theme.styles.action.border_width
                    border.color: root.vm.theme.styles.action.border

                    Text {
                        id: retryActionLabel
                        anchors.centerIn: parent
                        text: root.vm.status === null || root.vm.status.retry === null
                              ? "" : root.vm.status.retry.label
                        color: retryPress.pressed
                               ? root.vm.theme.styles.action.text_pressed
                               : root.vm.theme.styles.action.text
                        font.family: root.vm.theme.font.family
                        font.pixelSize: root.vm.theme.font.body
                    }

                    MouseArea {
                        id: retryPress
                        anchors.fill: parent
                        hoverEnabled: true
                        cursorShape: Qt.PointingHandCursor
                        onClicked: root.dispatchAction({ type: root.vm.status.retry.type })
                    }

                    Accessible.role: Accessible.Button
                    Accessible.name: retryActionLabel.text
                }
            }
        }
    }
}
