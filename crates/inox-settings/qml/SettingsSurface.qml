// The settings window's body: navigation, page, status line, actions, and the
// display trial's question on top of all of it.
//
// Everything is bound from `model`, the projection the Rust side computed. There
// is no colour, size or sentence in this file: colours come from `theme.colors`,
// control states from `theme.controls`, and every size from `theme.geometry`,
// which `src/theme.rs` derives from the design tokens.
//
// The three rules a reader should be able to check here are the ones acceptance
// D05, Q01 and Q02 ask for: a control that cannot be used says why, the keyboard's
// focus is visible on the control it is on, and walking the window with Tab
// brings the control into view (the projection says which container to scroll).
import QtQuick
import com.inox.settings.qml

FocusScope {
    id: surface

    // The projection, required: the window only loads this once it has one.
    required property var model
    // The Rust-backed element actions are sent to.
    required property var bridge
    // The page the window last drew, so a page change scrolls back to the top.
    property string shownPage: ""
    signal pageShown(string page)

    readonly property var theme: model.theme
    readonly property var geo: theme.geometry
    readonly property var colors: theme.colors
    readonly property var current: model.focus

    Accessible.role: Accessible.Pane
    Accessible.name: model.title

    // -- The window ------------------------------------------------------------

    Row {
        anchors.fill: parent
        spacing: 0

        // -- The navigation list (Q01: the walk follows the screen) -----------

        Rectangle {
            id: navFrame
            width: geo.navWidth
            height: parent.height
            color: colors.surface
            border.width: geo.hairline
            border.color: colors.border

            Flickable {
                id: nav
                anchors.fill: parent
                anchors.margins: geo.cardPadding
                contentWidth: width
                contentHeight: navColumn.implicitHeight
                clip: true
                boundsBehavior: Flickable.StopAtBounds

                Column {
                    id: navColumn
                    width: nav.width
                    spacing: geo.rowGap

                    Repeater {
                        id: navRepeater
                        model: surface.model.pages

                        delegate: ControlButton {
                            required property var modelData
                            width: navColumn.width
                            theme: surface.theme
                            label: modelData.label
                            selected: modelData.selected
                            focused: modelData.focused
                            focusId: modelData.focusId
                            onActivated: surface.bridge.dispatch(JSON.stringify({
                                type: "page-selected",
                                page: modelData.id
                            }))
                        }
                    }
                }
            }
        }

        // -- The page ---------------------------------------------------------

        Item {
            width: parent.width - navFrame.width
            height: parent.height

            Column {
                anchors.fill: parent
                anchors.margins: geo.pagePadding
                spacing: geo.sectionGap

                Flickable {
                    id: content
                    width: parent.width
                    height: parent.height - status.height - actions.height - 2 * parent.spacing
                    contentWidth: width
                    contentHeight: pageColumn.implicitHeight
                    clip: true
                    boundsBehavior: Flickable.StopAtBounds

                    Column {
                        id: pageColumn
                        width: content.width
                        spacing: geo.sectionGap

                        Text {
                            width: parent.width
                            text: surface.model.page.title
                            color: colors.text
                            font.family: theme.fontFamily
                            font.pixelSize: geo.titleSize
                            font.weight: geo.emphasisWeight
                            wrapMode: Text.WordWrap

                            Accessible.role: Accessible.Heading
                            Accessible.name: surface.model.page.title
                        }

                        Text {
                            width: parent.width
                            text: surface.model.page.description
                            color: colors.textMuted
                            font.family: theme.fontFamily
                            font.pixelSize: geo.captionSize
                            wrapMode: Text.WordWrap

                            Accessible.role: Accessible.StaticText
                            Accessible.name: surface.model.page.description
                        }

                        // Why the page cannot be worked, said once above the
                        // cards rather than beside every control (ui-guidelines
                        // §5, §6): the sentence is the same for all of them.
                        Repeater {
                            model: surface.model.page.notices

                            delegate: Text {
                                required property var modelData
                                width: pageColumn.width
                                text: modelData
                                color: colors.textMuted
                                font.family: theme.fontFamily
                                font.pixelSize: geo.captionSize
                                wrapMode: Text.WordWrap

                                Accessible.role: Accessible.StaticText
                                Accessible.name: modelData
                            }
                        }

                        Repeater {
                            model: surface.model.groups

                            delegate: Rectangle {
                                required property var modelData
                                width: pageColumn.width
                                implicitHeight: cardColumn.implicitHeight + 2 * geo.cardPadding
                                color: colors.surface
                                radius: geo.cardRadius
                                border.width: geo.hairline
                                border.color: colors.border

                                Column {
                                    id: cardColumn
                                    anchors.left: parent.left
                                    anchors.right: parent.right
                                    anchors.top: parent.top
                                    anchors.margins: geo.cardPadding
                                    spacing: geo.rowGap

                                    Text {
                                        width: parent.width
                                        text: modelData.title
                                        color: colors.text
                                        font.family: theme.fontFamily
                                        font.pixelSize: geo.headingSize
                                        font.weight: geo.emphasisWeight
                                        wrapMode: Text.WordWrap

                                        Accessible.role: Accessible.Heading
                                        Accessible.name: modelData.title
                                    }

                                    Repeater {
                                        model: modelData.fields

                                        delegate: FieldRow {
                                            required property var modelData
                                            width: cardColumn.width - 2 * geo.cardPadding
                                            field: modelData
                                            theme: surface.theme
                                            bridge: surface.bridge
                                            focusedId: surface.current.id
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // -- The status line: saved, waiting, in effect, refused ------

                StatusStrip {
                    id: status
                    width: parent.width
                    status: surface.model.status
                    theme: surface.theme
                }

                // -- The actions at the foot of the window --------------------

                Row {
                    id: actions
                    width: parent.width
                    height: geo.controlHeight
                    spacing: geo.inlineGap
                    layoutDirection: Qt.LeftToRight

                    Repeater {
                        model: surface.model.actions

                        delegate: ControlButton {
                            required property var modelData
                            theme: surface.theme
                            label: modelData.label
                            focused: modelData.focused
                            enabled: modelData.enabled
                            danger: modelData.danger
                            selected: modelData.primary
                            reason: modelData.disabledReason
                            onActivated: surface.bridge.dispatch(JSON.stringify({
                                type: modelData.id
                            }))
                        }
                    }
                }
            }
        }
    }

    // -- The display trial, on top of the window it may be about ----

    TrialDialog {
        anchors.fill: parent
        visible: surface.model.trial !== null
        trial: surface.model.trial
        theme: surface.theme
        bridge: surface.bridge
    }

    // -- The keyboard (Q01) -----------------------------------------------------

    // The projection says where the focus is and which container has to reveal
    // it; this walks the tree for the item that carries that identifier. The tree
    // never decides where the focus goes — Tab is a message to Rust.
    onCurrentChanged: surface.reveal(surface.current)
    onModelChanged: surface.followPage()

    function reveal(focus) {
        if (focus === null || focus.scroll === null || focus.id === null) {
            return
        }

        var container = focus.scroll === "navigation" ? nav : content
        var column = focus.scroll === "navigation" ? navColumn : pageColumn
        var item = surface.findFocus(column, focus.id)

        if (item === null) {
            return
        }

        var top = item.mapToItem(column, 0, 0).y
        var bottom = top + item.height

        if (top < container.contentY) {
            container.contentY = top
        } else if (bottom > container.contentY + container.height) {
            container.contentY = bottom - container.height
        }
    }

    // A new page starts at its own top, so the title is the first thing read.
    function followPage() {
        if (surface.model.page.id !== surface.shownPage) {
            content.contentY = 0
            surface.pageShown(surface.model.page.id)
        }
    }

    function findFocus(item, id) {
        if (item.focusId !== undefined && item.focusId === id) {
            return item
        }

        var children = item.children

        for (var index = 0; index < children.length; ++index) {
            var found = surface.findFocus(children[index], id)

            if (found !== null) {
                return found
            }
        }

        return null
    }

    // The loaded tree reports back what it built: the title it read, how many
    // pages it drew, and how many items expose an accessibility role. A tree that
    // lost a control or a role fails the offscreen check instead of passing on
    // "a URL was loaded" (root AGENTS.md §7.4).
    Component.onCompleted: {
        surface.bridge.reportReady(surface.model.title,
                                    navRepeater.count,
                                    surface.accessibleItemCount(surface))
    }

    function accessibleItemCount(item) {
        var count = item.Accessible.role !== undefined
                && item.Accessible.role !== Accessible.NoRole ? 1 : 0
        var children = item.children

        for (var index = 0; index < children.length; ++index) {
            count += surface.accessibleItemCount(children[index])
        }

        return count
    }
}
