// The Inox wallpaper preview.
//
// This tree only renders. Every colour, radius, spacing, font size and every
// user-visible string arrives in `app.modelJson`, projected from the design
// tokens and the Fluent catalogs in Rust (root AGENTS.md §4 and §7.3). Do not
// add literal colours, pixel sizes or text here — add them to `src/view.rs`.
// A line added below also has to be counted by
// `WallpaperViewModel::accessible_items`; the headless self-check fails when the
// two disagree.
import QtQuick
import QtQuick.Window
import com.inox.wallpaper.qml

Window {
    id: root

    // One JSON document, republished by Rust whenever the state changes.
    property var vm: JSON.parse(app.modelJson)

    // The drawing for the notice's control: the one place in this tree whose
    // appearance answers the pointer. It only chooses *which* of the four
    // drawings Rust projected applies (see `view::action_values`); it never
    // spells a colour itself.
    //
    // It sits on the root because both the control and the label inside it bind
    // to it. A property declared on the control is only reachable unqualified
    // from the control's own bindings, so the label's `retryStyle.text` was a
    // ReferenceError once QML compiled the file.
    property var retryStyle: {
        var notice = root.vm.notice
        if (notice === null || notice === undefined)
            return root.vm.tokens.card
        var styles = notice.action.styles
        if (retryMouse.pressed && notice.action.enabled)
            return styles.pressed
        if (retryMouse.containsMouse && notice.action.enabled)
            return styles.hovered
        return notice.action.style
    }

    width: vm.tokens.window.width
    height: vm.tokens.window.height
    visible: true
    title: vm.title
    color: vm.tokens.colors.background

    // The Rust-backed element: the state machine, the queues and the projection.
    // QML reports actions back through `dispatch` as JSON.
    WallpaperPreview {
        id: app
    }

    // The window has no calloop loop of its own: this timer is the pump that delivers finished
    // decodes and configuration reloads to the state machine.
    Timer {
        interval: 16
        running: true
        repeat: true
        onTriggered: app.pump()
    }

    // The readiness evidence: report once this tree has been built against the
    // projection and Qt has decoded the wallpaper image, if there is one.
    Timer {
        id: readiness
        interval: 16
        running: true
        repeat: true
        onTriggered: root.reportWhenSettled()
    }

    function reportWhenSettled() {
        var wallpaper = root.vm.wallpaper
        var imageReady = true
        if (wallpaper !== null && wallpaper !== undefined) {
            if (wallpaperImage.status === Image.Null || wallpaperImage.status === Image.Loading)
                return
            imageReady = wallpaperImage.status === Image.Ready
        }
        if (app.reportReady(root.vm.title, root.accessibleItemCount(content), imageReady))
            readiness.stop()
    }

    // Count the items that expose an accessibility role, so the headless check
    // can assert the semantic tree exists instead of trusting that it does. An
    // invisible item prunes its whole subtree.
    function accessibleItemCount(item) {
        if (!item.visible)
            return 0
        var count = (item.Accessible && item.Accessible.role !== Accessible.NoRole) ? 1 : 0
        var children = item.children
        for (var index = 0; index < children.length; ++index) {
            count += root.accessibleItemCount(children[index])
        }
        return count
    }

    // One action, as the state machine parses it.
    function dispatchAction(id) {
        app.dispatch(JSON.stringify({ type: id }))
    }

    // The one control this surface offers, from the keyboard and the pointer.
    function dispatchRetry() {
        var notice = root.vm.notice
        if (notice === null || notice === undefined || !notice.action.enabled)
            return
        root.dispatchAction(notice.action.id)
    }

    Item {
        id: content
        anchors.fill: parent
        focus: true

        Accessible.role: Accessible.Pane
        Accessible.name: root.vm.title

        Keys.onEscapePressed: root.dispatchRetry()
        Keys.onReturnPressed: root.dispatchRetry()

        // The wallpaper, or nothing: the window colour is the theme background.
        // One image — Qt tiles it when the geometry says tile, and stretches it
        // to the projected rectangle otherwise, so QML never scales an image to
        // a shape the geometry did not choose.
        Image {
            id: wallpaperImage
            visible: root.vm.wallpaper !== null
            x: visible ? root.vm.wallpaper.rect.x : 0
            y: visible ? root.vm.wallpaper.rect.y : 0
            width: visible ? root.vm.wallpaper.rect.width : 0
            height: visible ? root.vm.wallpaper.rect.height : 0
            source: visible ? root.vm.wallpaper.dataUrl : ""
            fillMode: (visible && root.vm.wallpaper.tiled) ? Image.Tile : Image.Stretch
        }

        // The cards: what the output is drawing, then whatever has to be read.
        Column {
            id: cards
            x: root.vm.tokens.sizes.windowPadding
            y: root.vm.tokens.sizes.windowPadding
            width: root.vm.tokens.sizes.cardWidth
            spacing: root.vm.tokens.spacing.lg

            Rectangle {
                id: statusCard
                width: parent.width
                height: statusLines.implicitHeight + 2 * root.vm.tokens.sizes.cardPadding
                color: root.vm.tokens.card.background
                radius: root.vm.tokens.card.radius
                border.width: root.vm.tokens.card.borderWidth
                border.color: root.vm.tokens.card.border

                Column {
                    id: statusLines
                    x: root.vm.tokens.sizes.cardPadding
                    y: root.vm.tokens.sizes.cardPadding
                    width: parent.width - 2 * root.vm.tokens.sizes.cardPadding
                    spacing: root.vm.tokens.spacing.sm

                    Repeater {
                        model: root.vm.card.lines

                        delegate: Text {
                            required property var modelData

                            width: statusLines.width
                            text: modelData.text
                            color: modelData.color
                            font.family: root.vm.tokens.font.family
                            font.pixelSize: modelData.size
                            font.weight: modelData.weight
                            wrapMode: Text.WordWrap

                            Accessible.role: Accessible.StaticText
                            Accessible.name: modelData.text
                        }
                    }
                }
            }

            Rectangle {
                id: noticeCard
                width: parent.width
                height: noticeContent.implicitHeight + 2 * root.vm.tokens.sizes.cardPadding
                visible: root.vm.notice !== null
                color: root.vm.tokens.card.background
                radius: root.vm.tokens.card.radius
                border.width: root.vm.tokens.card.borderWidth
                border.color: root.vm.tokens.card.border

                Column {
                    id: noticeContent
                    x: root.vm.tokens.sizes.cardPadding
                    y: root.vm.tokens.sizes.cardPadding
                    width: parent.width - 2 * root.vm.tokens.sizes.cardPadding
                    spacing: root.vm.tokens.spacing.lg

                    Column {
                        id: noticeStory
                        width: parent.width
                        spacing: root.vm.tokens.spacing.sm

                        Repeater {
                            model: root.vm.notice !== null ? root.vm.notice.lines : []

                            delegate: Text {
                                required property var modelData

                                width: noticeStory.width
                                text: modelData.text
                                color: modelData.color
                                font.family: root.vm.tokens.font.family
                                font.pixelSize: modelData.size
                                font.weight: modelData.weight
                                wrapMode: Text.WordWrap

                                Accessible.role: Accessible.StaticText
                                Accessible.name: modelData.text
                            }
                        }
                    }

                    Column {
                        id: noticeControls
                        width: parent.width
                        spacing: root.vm.tokens.spacing.sm

                        Rectangle {
                            id: retry
                            width: parent.width
                            height: root.vm.tokens.sizes.controlHeight
                            enabled: root.vm.notice !== null && root.vm.notice.action.enabled
                            color: root.retryStyle.background
                            radius: root.retryStyle.radius
                            border.width: root.retryStyle.borderWidth
                            border.color: root.retryStyle.border
                            activeFocusOnTab: true
                            focus: visible && enabled

                            Accessible.role: Accessible.Button
                            Accessible.name: root.vm.notice !== null ? root.vm.notice.action.label : ""
                            Accessible.description: (root.vm.notice !== null && root.vm.notice.action.reason !== null)
                                                    ? root.vm.notice.action.reason.text : ""
                            Accessible.focusable: true

                            Text {
                                anchors.centerIn: parent
                                text: root.vm.notice !== null ? root.vm.notice.action.label : ""
                                color: root.vm.notice !== null ? root.retryStyle.text : root.vm.tokens.colors.textMuted
                                font.family: root.vm.tokens.font.family
                                font.pixelSize: root.vm.notice !== null ? root.vm.notice.action.size : root.vm.tokens.font.size.body
                                font.weight: root.vm.notice !== null ? root.vm.notice.action.weight : root.vm.tokens.font.weight.normal
                            }

                            MouseArea {
                                id: retryMouse
                                anchors.fill: parent
                                hoverEnabled: true
                                enabled: root.vm.notice !== null && root.vm.notice.action.enabled
                                cursorShape: Qt.PointingHandCursor
                                onClicked: root.dispatchRetry()
                            }

                            Keys.onReturnPressed: root.dispatchRetry()
                            Keys.onEnterPressed: root.dispatchRetry()
                            Keys.onSpacePressed: root.dispatchRetry()
                        }

                        // Why the control cannot be pressed, underneath it — the
                        // reason is never colour alone.
                        Text {
                            id: retryReason
                            visible: root.vm.notice !== null && root.vm.notice.action.reason !== null
                            width: parent.width
                            text: visible ? root.vm.notice.action.reason.text : ""
                            color: visible ? root.vm.notice.action.reason.color : root.vm.tokens.colors.textMuted
                            font.family: root.vm.tokens.font.family
                            font.pixelSize: visible ? root.vm.notice.action.reason.size : root.vm.tokens.font.size.caption
                            font.weight: visible ? root.vm.notice.action.reason.weight : root.vm.tokens.font.weight.normal
                            wrapMode: Text.WordWrap

                            Accessible.role: Accessible.StaticText
                            Accessible.name: text
                        }
                    }
                }
            }
        }

        // The strip at the foot of the preview: what kind of surface this is.
        Rectangle {
            id: footerStrip
            x: root.vm.tokens.sizes.windowPadding
            y: content.height - height - root.vm.tokens.sizes.windowPadding
            width: content.width - 2 * root.vm.tokens.sizes.windowPadding
            height: footerLine.implicitHeight + 2 * root.vm.tokens.sizes.stripPadding
            visible: root.vm.footer !== null
            color: root.vm.tokens.card.background
            radius: root.vm.tokens.card.radius
            border.width: root.vm.tokens.card.borderWidth
            border.color: root.vm.tokens.card.border

            Text {
                id: footerLine
                x: root.vm.tokens.sizes.stripPadding
                y: root.vm.tokens.sizes.stripPadding
                width: parent.width - 2 * root.vm.tokens.sizes.stripPadding
                text: root.vm.footer !== null ? root.vm.footer.text : ""
                color: root.vm.footer !== null ? root.vm.footer.color : root.vm.tokens.colors.textMuted
                font.family: root.vm.tokens.font.family
                font.pixelSize: root.vm.footer !== null ? root.vm.footer.size : root.vm.tokens.font.size.caption
                font.weight: root.vm.footer !== null ? root.vm.footer.weight : root.vm.tokens.font.weight.normal
                wrapMode: Text.WordWrap

                Accessible.role: Accessible.StaticText
                Accessible.name: text
            }
        }
    }
}
