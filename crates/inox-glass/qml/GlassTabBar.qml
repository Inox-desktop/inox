// A glass tab bar: one pane, an emphasised tab for the current one, and a label
// per tab.
//
// A stateless renderer. `currentIndex` is the projection of the caller's state;
// selecting a tab is a click on a tab and therefore lives with the caller's hit
// testing, not here. Each tab exposes its own accessible role and name so the
// current tab is reachable from the accessibility tree even offscreen (the
// labels themselves come from Fluent, through the caller).
pragma ComponentBehavior: Bound

import QtQuick

Item {
    id: tabBar

    // The label per tab, in order. Fluent strings from the caller; empty in the
    // structural "no projection yet" case.
    property var labels: []
    // Which tab is current, as decided by the caller.
    property int currentIndex: 0
    // `enabled` is the inherited `Item.enabled`: the caller projects the state,
    // this component only exports it to the accessibility tree.
    // The tab bar's own accessible name (Fluent, from the caller).
    property string accessibleName: ""

    // Tokens: the pane, the current tab's tile, both hairlines and the labels.
    property color tint: "transparent"
    property color selectedTint: "transparent"
    property color highlight: "transparent"
    property color shade: "transparent"
    property color textColor: "transparent"
    property color selectedTextColor: "transparent"
    // Corner radius of the bar and of the current tab's tile.
    property real radius: 0
    property real hairline: 1
    // Font family and size from the `typography` tokens.
    property string fontFamily: ""
    property real fontSize: 0

    // Tabs divide the bar evenly; a bar with no tabs has no width to divide by.
    readonly property real tabWidth: tabBar.labels.length > 0
                                     ? tabBar.width / tabBar.labels.length
                                     : 0

    Accessible.role: Accessible.PageTabList
    Accessible.name: tabBar.accessibleName

    GlassPane {
        anchors.fill: parent
        tint: tabBar.tint
        highlight: tabBar.highlight
        shade: tabBar.shade
        radius: tabBar.radius
        hairline: tabBar.hairline
    }

    Repeater {
        model: tabBar.labels

        delegate: Item {
            id: tab

            required property int index
            required property string modelData

            x: tab.index * tabBar.tabWidth
            width: tabBar.tabWidth
            height: tabBar.height

            readonly property bool current: tab.index === tabBar.currentIndex

            Accessible.role: Accessible.PageTab
            Accessible.name: tab.modelData
            Accessible.selected: tab.current

            Rectangle {
                anchors.fill: parent
                visible: tab.current
                color: tabBar.selectedTint
                radius: tabBar.radius
                antialiasing: true
            }

            Text {
                anchors.fill: parent
                text: tab.modelData
                color: tab.current ? tabBar.selectedTextColor : tabBar.textColor
                font.family: tabBar.fontFamily
                font.pixelSize: tabBar.fontSize
                horizontalAlignment: Text.AlignHCenter
                verticalAlignment: Text.AlignVCenter
                elide: Text.ElideRight
            }
        }
    }
}
