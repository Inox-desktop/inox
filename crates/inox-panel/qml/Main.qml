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

    // The colour the window is *cleared* with. It is a host input rather than a
    // drawing: the projection's own background is painted by the canvas below,
    // so a host may swap the clear colour (the alpha reconstruction in
    // `src/offscreen.cpp` clears over black and over white) without the panel
    // losing anything it draws. `transparent` is the structural "no colour"
    // sentinel (`theme::TRANSPARENT`), not a design colour; the host replaces it
    // before every grab.
    property color hostBackdrop: "transparent"

    visible: true
    color: root.hostBackdrop

    Item {
        id: canvas
        width: root.width
        height: root.height

        // The projection's background, as content: it has to survive the host
        // swapping the clear colour, so it is a rectangle and not `color:`.
        Rectangle {
            anchors.fill: parent
            color: root.vm.background
        }

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

                // A frosted pane is the shared glass component, not a rectangle
                // the panel tints itself: `GlassPane` belongs to `inox-glass`,
                // whose assets this crate's `build.rs` compiles into the module
                // it registers, so the panel and every other consumer draw the
                // same material from the same tokens (ADR 0040 §3). The
                // projection carries the five values the component binds and the
                // panel decides none of them here.
                GlassPane {
                    anchors.fill: parent
                    visible: modelData.kind === "pane"
                    tint: modelData.color
                    highlight: modelData.highlight
                    shade: modelData.shade
                    hairline: modelData.hairline
                    highlighted: modelData.highlighted
                    shaded: modelData.shaded
                    radius: modelData.radius
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
