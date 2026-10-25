// The panel's QML renderer.
//
// This file is a *renderer*, not a view: the panel's input, focus, placement
// and state all live in Rust (`src/layer.rs`, `src/app.rs`), and what arrives
// here is one JSON projection per frame (`src/view.rs`) describing rectangles
// and text on absolute coordinates. Nothing in this file chooses a colour, a
// size or a sentence — `theme.rs` turns the design tokens into those values and
// `view.rs` lays them out.
//
// The root is a `Window` because `QQmlApplicationEngine` returns the
// `QQuickWindow` the C++ host grabs. Under `QT_QPA_PLATFORM=offscreen` it never
// becomes an `xdg_toplevel`, and it is never mapped by the compositor: the
// panel's real surface is the `wlr-layer-shell` one Rust owns.
import QtQuick
import QtQuick.Window

Window {
    id: root

    // The projection, and nothing else. `modelJson` starts as the structural
    // "nothing to paint yet" sentinel — the same transparent no-colour value
    // the Rust side uses — so the bindings below are never evaluated against a
    // missing object. The first projection replaces it before any frame is
    // grabbed.
    property string modelJson: '{"background":"transparent","items":[]}'

    readonly property var vm: JSON.parse(root.modelJson)

    visible: true
    color: root.vm.background

    Item {
        id: canvas
        width: root.width
        height: root.height

        Repeater {
            model: root.vm.items
            delegate: Item {
                x: modelData.x
                y: modelData.y
                width: modelData.w
                height: modelData.h
                clip: true

                Rectangle {
                    anchors.fill: parent
                    visible: modelData.kind === "rect"
                    color: modelData.color
                    radius: modelData.radius
                    border.width: modelData.border
                    border.color: modelData.border_color
                }

                Text {
                    anchors.fill: parent
                    visible: modelData.kind === "text"
                    text: modelData.text
                    color: modelData.color
                    font.family: root.vm.font_family
                    font.pixelSize: modelData.size
                    font.bold: modelData.bold
                    elide: Text.ElideRight
                    clip: true
                    verticalAlignment: Text.AlignVCenter
                    horizontalAlignment: modelData.align === "center" ? Text.AlignHCenter
                                       : modelData.align === "right" ? Text.AlignRight
                                       : Text.AlignLeft
                }
            }
        }
    }
}
