// The glass component catalog: all five components, from one projection.
//
// This is the page a host's offscreen QML test renders (ADR 0040 §3). The root is
// a `Window` because `QQmlApplicationEngine` hands the host the `QQuickWindow` it
// creates and the host grabs that window; under `QT_QPA_PLATFORM=offscreen` it is
// never mapped and never becomes a toplevel.
//
// Like every other L3 renderer, the page holds no colour, no size and no
// sentence: `modelJson` is one JSON projection per frame, produced by
// `inox_glass::catalog_model(&theme)` (`src/catalog.rs` documents the schema).
// Every rectangle carries its own geometry in the model, so a pixel test reads the
// coordinates it asserts on out of the projection instead of guessing a layout.
//
// The default is the structural "nothing projected yet" sentinel: a zero-sized
// page in which every component is transparent. A host replaces it before the
// first frame is grabbed, exactly as for the panel's `Main.qml`.
import QtQuick
import QtQuick.Window

Window {
    id: root

    property string modelJson: '{"width":0,"height":0,"background":"#00000000","fontFamily":"","fontSize":0,"hairline":0,"pane":{"x":0,"y":0,"width":0,"height":0,"radius":0,"tint":"transparent","highlight":"transparent","shade":"transparent"},"button":{"x":0,"y":0,"width":0,"height":0,"radius":0,"text":"","tint":"transparent","tintPressed":"transparent","highlight":"transparent","shade":"transparent","textColor":"transparent","focusRing":"transparent","focusRingWidth":0,"pressed":false,"focused":false,"enabled":true},"toggle":{"x":0,"y":0,"width":0,"height":0,"radius":0,"label":"","trackTint":"transparent","trackTintChecked":"transparent","knobColor":"transparent","checked":true,"enabled":true},"slider":{"x":0,"y":0,"width":0,"height":0,"trackRadius":0,"label":"","trackTint":"transparent","fillTint":"transparent","knobColor":"transparent","knobSize":0,"value":0,"enabled":true},"tabBar":{"x":0,"y":0,"width":0,"height":0,"radius":0,"accessibleName":"","labels":[],"currentIndex":0,"tint":"transparent","selectedTint":"transparent","highlight":"transparent","shade":"transparent","textColor":"transparent","selectedTextColor":"transparent","enabled":true}}'

    readonly property var model: JSON.parse(root.modelJson)

    // The clear colour is a host input, not a drawing: a host that has to
    // reconstruct the alpha channel renders the page over black and over white.
    // `transparent` here is the structural "no colour" sentinel.
    property color hostBackdrop: "transparent"

    visible: true
    color: root.hostBackdrop
    width: root.model.width
    height: root.model.height

    // The page's own fill, as content: it survives the host swapping the clear
    // colour, so a test can tell "the page painted this" from "the window cleared
    // this".
    Rectangle {
        anchors.fill: parent
        color: root.model.background
    }

    GlassPane {
        x: root.model.pane.x
        y: root.model.pane.y
        width: root.model.pane.width
        height: root.model.pane.height
        radius: root.model.pane.radius
        hairline: root.model.hairline
        tint: root.model.pane.tint
        highlight: root.model.pane.highlight
        shade: root.model.pane.shade
    }

    GlassButton {
        x: root.model.button.x
        y: root.model.button.y
        width: root.model.button.width
        height: root.model.button.height
        radius: root.model.button.radius
        hairline: root.model.hairline
        text: root.model.button.text
        fontFamily: root.model.fontFamily
        fontSize: root.model.fontSize
        tint: root.model.button.tint
        tintPressed: root.model.button.tintPressed
        highlight: root.model.button.highlight
        shade: root.model.button.shade
        textColor: root.model.button.textColor
        focusRing: root.model.button.focusRing
        focusRingWidth: root.model.button.focusRingWidth
        pressed: root.model.button.pressed
        focused: root.model.button.focused
        enabled: root.model.button.enabled
    }

    GlassToggle {
        x: root.model.toggle.x
        y: root.model.toggle.y
        width: root.model.toggle.width
        height: root.model.toggle.height
        radius: root.model.toggle.radius
        label: root.model.toggle.label
        trackTint: root.model.toggle.trackTint
        trackTintChecked: root.model.toggle.trackTintChecked
        knobColor: root.model.toggle.knobColor
        checked: root.model.toggle.checked
        enabled: root.model.toggle.enabled
    }

    GlassSlider {
        x: root.model.slider.x
        y: root.model.slider.y
        width: root.model.slider.width
        height: root.model.slider.height
        trackRadius: root.model.slider.trackRadius
        label: root.model.slider.label
        trackTint: root.model.slider.trackTint
        fillTint: root.model.slider.fillTint
        knobColor: root.model.slider.knobColor
        knobSize: root.model.slider.knobSize
        value: root.model.slider.value
        enabled: root.model.slider.enabled
    }

    GlassTabBar {
        x: root.model.tabBar.x
        y: root.model.tabBar.y
        width: root.model.tabBar.width
        height: root.model.tabBar.height
        radius: root.model.tabBar.radius
        hairline: root.model.hairline
        accessibleName: root.model.tabBar.accessibleName
        labels: root.model.tabBar.labels
        currentIndex: root.model.tabBar.currentIndex
        tint: root.model.tabBar.tint
        selectedTint: root.model.tabBar.selectedTint
        highlight: root.model.tabBar.highlight
        shade: root.model.tabBar.shade
        textColor: root.model.tabBar.textColor
        selectedTextColor: root.model.tabBar.selectedTextColor
        fontFamily: root.model.fontFamily
        fontSize: root.model.fontSize
        enabled: root.model.tabBar.enabled
    }
}
