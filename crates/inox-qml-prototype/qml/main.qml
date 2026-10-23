import QtQuick
import QtQuick.Controls
import com.inox.prototype

Window {
    id: root
    width: 480
    height: 320
    visible: true
    title: "Inox QML Prototype"

    // The Rust-backed QObject exposed through cxx-qt. Its `number` and `string`
    // Q_PROPERTYs and the `incrementNumber` invokable live in `MyObjectRust`.
    MyObject {
        id: counter
    }

    Rectangle {
        anchors.centerIn: parent
        width: 360
        height: 200
        radius: 12
        color: "#2b2b2b"

        // AT-SPI accessibility: expose this surface and its label roles so
        // screen readers / the Inox accessibility bridge can traverse them.
        Accessible.role: Accessible.Pane
        Accessible.name: "Prototype window"

        Column {
            anchors.centerIn: parent
            spacing: 16

            Text {
                id: numberLabel
                anchors.horizontalCenter: parent.horizontalCenter
                color: "white"
                font.pixelSize: 32
                text: counter.number

                Accessible.role: Accessible.StaticText
                Accessible.name: "Current number"
            }

            Text {
                anchors.horizontalCenter: parent.horizontalCenter
                color: "#cccccc"
                font.pixelSize: 16
                text: "string: " + counter.string
            }

            Button {
                anchors.horizontalCenter: parent.horizontalCenter
                text: "Increment"

                onClicked: {
                    counter.incrementNumber()
                    counter.string = "Clicked " + counter.number + " time(s)"
                }

                Accessible.role: Accessible.Button
                Accessible.name: "Increment number"
            }
        }
    }
}