// The Inox login preview (ADR 0036 M03).
//
// This tree only renders: every colour, size and string comes from the
// Rust-backed `GreetPreview` element below, which projects the design tokens
// and the Fluent catalogs (root AGENTS.md §4 and §7.3). Do not add literal
// colours, pixel sizes or user-visible text here — add them to
// `src/view.rs` instead.
//
// The surface cannot authenticate: the login control is disabled, and the
// element drops every command the state machine returns because the shipped
// binary has no reviewed transport attached (ADR 0015).
import QtQuick
import QtQuick.Window
import com.inox.greet.qml

Window {
    id: root

    width: preview.windowWidth
    height: preview.windowHeight
    visible: true
    title: preview.title
    color: preview.background

    // The Rust-backed projection of the theme and the catalogs.
    GreetPreview {
        id: preview
    }

    // The self-check waits for this: it proves the engine instantiated the
    // Rust element, that its properties crossed the bridge intact, and that the
    // semantic tree a screen reader walks is actually populated (root
    // AGENTS.md §7.4). `reportReady` only accepts the report when both the title
    // and the accessible-item count agree with the projection.
    Component.onCompleted: preview.reportReady(preview.title, root.accessibleItemCount(content))

    // Count the items that expose an accessibility role, so the headless check
    // can assert the semantic tree exists instead of trusting that it does.
    function accessibleItemCount(item) {
        var count = item.Accessible.role !== undefined
                && item.Accessible.role !== Accessible.NoRole ? 1 : 0
        var children = item.children
        for (var i = 0; i < children.length; ++i) {
            count += root.accessibleItemCount(children[i])
        }
        return count
    }

    // Escape clears the identity and returns focus to the field, matching the
    // keyboard order of the Iced preview it replaces.
    function clearIdentity() {
        usernameField.text = ""
        preview.clearIdentity()
        usernameField.forceActiveFocus()
    }

    FocusScope {
        id: content
        anchors.fill: parent
        anchors.margins: preview.spacingLg

        // `Accessible` attaches to Items only, so the role lives on the content
        // surface rather than on the Window itself.
        Accessible.role: Accessible.Pane
        Accessible.name: preview.title

        Keys.onEscapePressed: root.clearIdentity()

        Column {
            id: fields
            anchors.left: parent.left
            anchors.right: parent.right
            spacing: preview.spacingMd

            Text {
                width: parent.width
                text: preview.title
                color: preview.text
                font.pixelSize: preview.fontSize
                wrapMode: Text.WordWrap

                Accessible.role: Accessible.Heading
                Accessible.name: preview.title
            }

            Text {
                width: parent.width
                text: preview.previewNotice
                color: preview.warning
                font.pixelSize: preview.fontSize
                wrapMode: Text.WordWrap

                Accessible.role: Accessible.StaticText
                Accessible.name: preview.previewNotice
            }

            Text {
                width: parent.width
                text: preview.userLabel
                color: preview.text
                font.pixelSize: preview.fontSize

                Accessible.role: Accessible.StaticText
                Accessible.name: preview.userLabel
            }

            // The account-name field. It collects a public account name only;
            // the preview never asks for or stores a password.
            Rectangle {
                id: field
                width: parent.width
                height: preview.fontSize + 4 * preview.spacingSm
                radius: preview.radiusMd
                color: preview.surface
                border.width: 1
                border.color: usernameField.activeFocus ? preview.accent : preview.border

                TextInput {
                    id: usernameField
                    anchors.fill: parent
                    anchors.margins: preview.spacingSm
                    verticalAlignment: TextInput.AlignVCenter
                    color: preview.text
                    font.pixelSize: preview.fontSize
                    selectionColor: preview.accent
                    selectedTextColor: preview.accentText
                    activeFocusOnTab: true
                    onTextEdited: preview.editUsername(text)
                    onAccepted: preview.activate()

                    Accessible.role: Accessible.EditableText
                    Accessible.name: preview.userLabel
                    Accessible.description: preview.previewNotice

                    Component.onCompleted: forceActiveFocus()
                }

                // A localised label shown while the field is empty, so the
                // surface never needs a hard-coded placeholder string.
                Text {
                    anchors.fill: usernameField
                    anchors.margins: preview.spacingSm
                    verticalAlignment: Text.AlignVCenter
                    visible: usernameField.text.length === 0 && !usernameField.activeFocus
                    text: preview.userLabel
                    color: preview.textMuted
                    font.pixelSize: preview.fontSize
                    elide: Text.ElideRight
                }
            }

            Text {
                width: parent.width
                text: preview.notice
                color: preview.textMuted
                font.pixelSize: preview.fontSize
                wrapMode: Text.WordWrap

                Accessible.role: Accessible.StaticText
                Accessible.name: preview.notice
            }

            // Login is permanently disabled while no reviewed transport exists.
            Rectangle {
                width: parent.width
                height: preview.fontSize + 2 * preview.spacingSm
                radius: preview.radiusMd
                color: preview.surfaceVariant

                Text {
                    anchors.centerIn: parent
                    text: preview.loginLabel
                    color: preview.textMuted
                    font.pixelSize: preview.fontSize
                }

                Accessible.role: Accessible.Button
                Accessible.name: preview.loginLabel
                Accessible.focusable: false
            }

            // Clear drops the edited identity; Escape and Enter do the same.
            Rectangle {
                id: clearButton
                width: parent.width
                height: preview.fontSize + 2 * preview.spacingSm
                radius: preview.radiusMd
                color: clearButton.activeFocus ? preview.accent : preview.surfaceVariant

                Text {
                    anchors.centerIn: parent
                    text: preview.clearLabel
                    color: clearButton.activeFocus ? preview.accentText : preview.text
                    font.pixelSize: preview.fontSize
                }

                activeFocusOnTab: true

                Accessible.role: Accessible.Button
                Accessible.name: preview.clearLabel

                Keys.onReturnPressed: root.clearIdentity()
                Keys.onEnterPressed: root.clearIdentity()
                Keys.onSpacePressed: root.clearIdentity()
            }
        }
    }
}
