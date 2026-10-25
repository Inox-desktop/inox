// One control of the settings window: a page entry, an option, a switch, a step
// button, an action.
//
// Its five states are drawn from the theme's control table, which `src/theme.rs`
// resolves from the design tokens: resting, hovered, pressed, focused, selected,
// selected-and-focused, disabled, and the destructive variants. No colour and no
// size is written here — a literal would be a value the tokens own
// (ui-guidelines §1, §5).
import QtQuick

Item {
    id: button

    // The projection's theme.
    required property var theme
    // What the control says.
    property string label: ""
    // Whether it holds the current value.
    property bool selected: false
    // Whether the keyboard is on it.
    property bool focused: false
    // Whether it throws something away, and is drawn in the danger colour.
    property bool danger: false
    // "Whether it can be used" is `Item.enabled`, which a control inherits
    // rather than shadowing: a redeclared `enabled` would stop the tree's own
    // input handling from seeing it (and Qt warns that it overrides a base
    // member). The states below read the inherited property.
    // Why it cannot be, when it cannot (shown by the container that owns it).
    property string reason: ""
    // The identifier the window's own focus uses for this control.
    property string focusId: ""

    signal activated()

    readonly property var geo: theme.geometry
    readonly property var style: button.styleFor()
    readonly property alias hovered: mouse.containsMouse

    implicitWidth: labelText.implicitWidth + 2 * geo.controlPaddingHorizontal
    implicitHeight: Math.max(geo.controlHeight,
                             labelText.implicitHeight + 2 * geo.controlPaddingVertical)

    Rectangle {
        anchors.fill: parent
        radius: button.geo.radius
        color: button.style.background
        border.width: button.style.borderWidth
        border.color: button.style.border
    }

    Text {
        id: labelText
        anchors.centerIn: parent
        width: Math.min(implicitWidth, button.width - 2 * button.geo.controlPaddingHorizontal)
        text: button.label
        color: button.style.foreground
        font.family: button.theme.fontFamily
        font.pixelSize: button.geo.bodySize
        horizontalAlignment: Text.AlignHCenter
        wrapMode: Text.WrapAtWordBoundaryOrAnywhere

        Accessible.role: Accessible.Button
        Accessible.name: button.label
        Accessible.focusable: button.enabled
    }

    MouseArea {
        id: mouse
        anchors.fill: parent
        hoverEnabled: true
        enabled: button.enabled
        cursorShape: Qt.PointingHandCursor
        onClicked: button.activated()
    }

    // The five states, resolved from the theme's table. Hovering the current
    // option keeps its accent background, so the on-accent text stays readable.
    function styleFor() {
        var states = button.theme.controls

        if (!button.enabled) {
            return states.disabled
        }

        if (mouse.pressed) {
            if (button.danger) {
                return states.dangerPressed
            }

            return button.focused ? states.pressedFocused : states.pressed
        }

        if (button.danger) {
            if (mouse.containsMouse) {
                return button.focused ? states.dangerHoveredFocused : states.dangerHovered
            }

            return button.focused ? states.dangerFocused : states.danger
        }

        if (button.selected) {
            return button.focused ? states.selectedFocused : states.selected
        }

        if (mouse.containsMouse) {
            return button.focused ? states.plainHoveredFocused : states.plainHovered
        }

        return button.focused ? states.plainFocused : states.plain
    }
}
