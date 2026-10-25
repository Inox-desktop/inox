// The lock screen's interface, as QML.
//
// This file is a pure renderer: it makes no decisions, holds no
// state, and reads no input. Everything it draws — every word, every colour,
// every distance — comes from `bridge.modelJson`, the projection
// `crate::view` builds in Rust from `LockViewModel` and `inox_config::Theme`.
// There is no literal colour, pixel size or user-visible English string here,
// by contract (§4).
//
// The one value that never arrives is the credential: `field.value` is the mask
// the projection built out of its *length*, so a screenshot of this surface has
// nothing to redact.
//
// `surfaceWidth` / `surfaceHeight` are context properties the C++ host sets
// before loading this file: a window needs an initial size and this crate does
// not write pixel sizes in QML. Later frames are resized by the host, which owns
// the surface size the compositor configured (contract §4, ).

import QtQuick
import com.inox.lock.qml

Window {
    id: root
    visible: true
    width: surfaceWidth
    height: surfaceHeight
    color: root.screen ? root.screen.palette.scrim : Qt.rgba(0, 0, 0, 0)

    // The projection. Null until the host has pushed the first frame's model.
    property var screen: bridge.modelJson.length > 0 ? JSON.parse(bridge.modelJson) : null

    // The Rust-backed object the projection arrives through.
    LockScreen {
        id: bridge
        objectName: "lockBridge"
    }

    // Instantiated only once there is a projection to draw, so no binding reads
    // a property that is not there yet.
    Loader {
        anchors.fill: parent
        active: root.screen !== null
        sourceComponent: Component {
            Item {
                anchors.fill: parent
                Accessible.role: Accessible.Pane
                Accessible.name: root.screen.headline.text

                // The card: `surface` over the full-bleed scrim, a hairline
                // outline, and `md` corners (guidelines §4).
                Rectangle {
                    id: card
                    anchors.centerIn: parent
                    width: root.width * root.screen.metrics.cardWidthFraction
                    height: content.implicitHeight + root.screen.metrics.cardPadding * 2
                    radius: root.screen.metrics.cardRadius
                    color: root.screen.palette.card
                    border.width: root.screen.metrics.hairline
                    border.color: root.screen.palette.cardBorder

                    Column {
                        id: content
                        anchors.centerIn: parent
                        width: parent.width - root.screen.metrics.cardPadding * 2
                        spacing: root.screen.metrics.spacingLg

                        // What the screen is and what it has to say: a heading
                        // and the lines that explain or report under it.
                        Column {
                            id: head
                            width: parent.width
                            spacing: root.screen.metrics.spacingSm

                            Text {
                                width: parent.width
                                wrapMode: Text.WordWrap
                                text: root.screen.headline.text
                                font.family: root.screen.fonts.family
                                font.pixelSize: root.screen.headline.size
                                color: root.screen.headline.color
                                Accessible.role: Accessible.StaticText
                                Accessible.name: root.screen.headline.text
                            }

                            Repeater {
                                model: root.screen.lines
                                Text {
                                    width: head.width
                                    wrapMode: Text.WordWrap
                                    text: modelData.text
                                    font.family: root.screen.fonts.family
                                    font.pixelSize: modelData.size
                                    color: modelData.color
                                }
                            }
                        }

                        // The credential field. It is drawn only where the
                        // projection says this screen asks for a credential at
                        // all — a screen that explains it cannot check a password
                        // must not also hold out a password box.
                        Rectangle {
                            id: field
                            visible: root.screen.field.visible
                            width: content.width * root.screen.metrics.controlWidthFraction
                            height: fieldColumn.implicitHeight + root.screen.metrics.controlPadding * 2
                            radius: root.screen.metrics.controlRadius
                            color: root.screen.field.background
                            border.width: root.screen.metrics.hairline
                            border.color: root.screen.field.border

                            Column {
                                id: fieldColumn
                                anchors.centerIn: parent
                                width: parent.width - root.screen.metrics.controlPadding * 2
                                spacing: root.screen.metrics.spacingSm

                                Text {
                                    width: parent.width
                                    text: root.screen.field.label
                                    font.family: root.screen.fonts.family
                                    font.pixelSize: root.screen.field.labelSize
                                    color: root.screen.field.labelColor
                                    Accessible.role: Accessible.StaticText
                                    Accessible.name: root.screen.field.label
                                }

                                Text {
                                    width: parent.width
                                    wrapMode: Text.NoWrap
                                    elide: Text.ElideRight
                                    text: root.screen.field.value
                                    font.family: root.screen.fonts.family
                                    font.pixelSize: root.screen.field.valueSize
                                    color: root.screen.field.valueColor
                                }
                            }
                        }

                        // The reason a control cannot be used, the controls, and
                        // the hint about the key that reaches them.
                        Column {
                            id: actions
                            width: parent.width
                            spacing: root.screen.metrics.spacingSm

                            Repeater {
                                model: root.screen.notices
                                Text {
                                    width: actions.width
                                    wrapMode: Text.WordWrap
                                    text: modelData.text
                                    font.family: root.screen.fonts.family
                                    font.pixelSize: modelData.size
                                    color: modelData.color
                                }
                            }

                            Repeater {
                                model: root.screen.actions
                                Rectangle {
                                    width: content.width * root.screen.metrics.controlWidthFraction
                                    height: actionLabel.implicitHeight + root.screen.metrics.controlPadding * 2
                                    radius: modelData.radius
                                    color: modelData.background
                                    border.width: modelData.borderWidth
                                    border.color: modelData.border
                                    Accessible.role: Accessible.Button
                                    Accessible.name: modelData.label

                                    Text {
                                        id: actionLabel
                                        anchors.centerIn: parent
                                        text: modelData.label
                                        font.family: root.screen.fonts.family
                                        font.pixelSize: modelData.size
                                        color: modelData.color
                                    }
                                }
                            }

                            Text {
                                visible: root.screen.hint !== null
                                width: actions.width
                                wrapMode: Text.WordWrap
                                text: root.screen.hint ? root.screen.hint.text : ""
                                font.family: root.screen.fonts.family
                                font.pixelSize: root.screen.hint ? root.screen.hint.size : 0
                                color: root.screen.hint ? root.screen.hint.color : Qt.rgba(0, 0, 0, 0)
                            }
                        }
                    }
                }
            }
        }
    }
}
