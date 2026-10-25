// The Inox settings window.
//
// This file is the window chrome and nothing else: it creates the Rust-backed
// `SettingsApp`, hands the parsed projection to the surface, and owns the two
// pieces of timing a declarative tree has to own — the pump timer and the keys
// the window answers itself.
//
// Nothing here holds a colour, a size or a sentence. Every value the window draws
// is in the projection the Rust side computed from the design tokens and the
// Fluent catalogs (root AGENTS.md §4 and §7.3), and the window's own geometry,
// title and background are set from that projection too. A literal in this file
// would be a defect the projection tests catch: see
// `src/headless_tests/projection.rs`.
import QtQuick
import QtQuick.Window
import com.inox.settings.qml

Window {
    id: root

    // The Rust-backed projection. QML renders it; it never decides anything.
    SettingsApp {
        id: app
    }

    // The window's model, parsed from the projection.
    readonly property var model: root.parsed()
    readonly property bool ready: root.model !== null

    // The page the content last showed, so a page change scrolls back to the top.
    property string shownPage: ""

    visible: false

    // The native close button asks the runtime first, then ends the event loop.
    onClosing: {
        app.quit()
        Qt.quit()
    }

    function parsed() {
        try {
            var value = JSON.parse(app.modelJson)
            return value && value.theme ? value : null
        } catch (error) {
            return null
        }
    }

    function applyModel() {
        var value = root.parsed()

        if (value === null) {
            return
        }

        root.color = value.theme.colors.window
        root.title = value.title
        root.width = value.width
        root.height = value.height
        root.visible = true
    }

    Component.onCompleted: root.applyModel()

    Connections {
        target: app
        function onModelJsonChanged() { root.applyModel() }
    }

    // The pump: answers from work that finished on another thread, and the
    // display trial's countdown. Sixteen milliseconds is what makes a
    // save that settled off-thread reach the status line without a callback
    // crossing the FFI boundary.
    Timer {
        interval: 16
        running: true
        repeat: true
        onTriggered: app.pump()
    }

    // The keys the window answers itself: the focus walk, activation, and
    // unwinding a level. Every other key belongs to the control that has the text
    // cursor, which is why an unrecognised key is not reported at all.
    //
    // `Keys` is an attached property of `Item`, and the window's root is not one:
    // attached to the `Window` it is silently dropped and the window would answer
    // no key at all. So the content lives in an Item that holds the keyboard, and
    // the surface is loaded inside it.
    Item {
        anchors.fill: parent
        // The window's own focus starts here; a field that takes the text cursor
        // takes it from here, and a key it does not use walks back up to this
        // handler.
        focus: true

        Keys.onPressed: function (event) {
            var name = root.keyOf(event.key)

            if (name === "") {
                return
            }

            app.dispatch(JSON.stringify({
                type: "key",
                key: name,
                shift: (event.modifiers & Qt.ShiftModifier) !== 0,
                repeat: event.isAutoRepeat
            }))
            event.accepted = true
        }

        // The surface exists only once there is a projection to render: the
        // binary installs the runtime before this module is loaded, so this
        // guards a broken projection rather than a state a user ever sees.
        Loader {
            id: loader
            anchors.fill: parent
            active: root.ready
            sourceComponent: surface
        }
    }

    function keyOf(key) {
        if (key === Qt.Key_Tab) {
            return "Tab"
        }
        if (key === Qt.Key_Return || key === Qt.Key_Enter) {
            return "Enter"
        }
        if (key === Qt.Key_Escape) {
            return "Escape"
        }
        if (key === Qt.Key_Down) {
            return "ArrowDown"
        }
        if (key === Qt.Key_Up) {
            return "ArrowUp"
        }

        return ""
    }

    // The surface exists only once there is a projection to render: the binary
    // installs the runtime before this module is loaded, so this guards a broken
    // projection rather than a state a user ever sees.
    Component {
        id: surface

        SettingsSurface {
            anchors.fill: parent
            model: root.model
            bridge: app
            shownPage: root.shownPage
            onPageShown: function (page) { root.shownPage = page }
        }
    }
}
