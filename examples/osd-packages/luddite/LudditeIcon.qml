// Vector icons for the Luddite OSD: a microphone while dictating, a
// crossed-out microphone while the default input is muted, and an
// hourglass while transcribing. Drawn with QtQuick.Shapes so no Nerd
// Font is needed. Paths are authored on a 24x24 grid; the microphone
// matches voxtype-shared/StateIcon.qml.

import QtQuick
import QtQuick.Shapes

Item {
    id: root

    /// "mic", "muted" or "hourglass".
    property string kind: "mic"
    property color color: "#dcdfe4"
    property real strokeWidth: 2.0

    implicitWidth: 24
    implicitHeight: 24

    Item {
        width: 24
        height: 24
        anchors.centerIn: parent
        scale: Math.min(root.width, root.height) / 24

        Shape {
            anchors.fill: parent
            visible: root.kind === "mic" || root.kind === "muted"
            preferredRendererType: Shape.CurveRenderer
            ShapePath {
                strokeColor: root.color
                strokeWidth: root.strokeWidth
                fillColor: "transparent"
                capStyle: ShapePath.RoundCap
                joinStyle: ShapePath.RoundJoin
                PathRectangle { x: 9; y: 2.5; width: 6; height: 11.5; radius: 3 }
            }
            ShapePath {
                strokeColor: root.color
                strokeWidth: root.strokeWidth
                fillColor: "transparent"
                capStyle: ShapePath.RoundCap
                joinStyle: ShapePath.RoundJoin
                startX: 5.5; startY: 10.5
                PathLine { x: 5.5; y: 11.5 }
                PathArc { x: 18.5; y: 11.5; radiusX: 6.5; radiusY: 6.5; direction: PathArc.Counterclockwise }
                PathLine { x: 18.5; y: 10.5 }
                PathMove { x: 12; y: 18 }
                PathLine { x: 12; y: 21.5 }
            }
        }

        Shape {
            anchors.fill: parent
            visible: root.kind === "muted"
            preferredRendererType: Shape.CurveRenderer
            ShapePath {
                strokeColor: root.color
                strokeWidth: root.strokeWidth
                fillColor: "transparent"
                capStyle: ShapePath.RoundCap
                startX: 3.5; startY: 3.5
                PathLine { x: 20.5; y: 20.5 }
            }
        }

        Shape {
            anchors.fill: parent
            visible: root.kind === "hourglass"
            preferredRendererType: Shape.CurveRenderer
            ShapePath {
                strokeColor: root.color
                strokeWidth: root.strokeWidth
                fillColor: "transparent"
                capStyle: ShapePath.RoundCap
                joinStyle: ShapePath.RoundJoin
                startX: 6; startY: 3
                PathLine { x: 18; y: 3 }
                PathMove { x: 6; y: 21 }
                PathLine { x: 18; y: 21 }
                PathMove { x: 7.5; y: 3 }
                PathLine { x: 7.5; y: 7 }
                PathLine { x: 12; y: 12 }
                PathLine { x: 7.5; y: 17 }
                PathLine { x: 7.5; y: 21 }
                PathMove { x: 16.5; y: 3 }
                PathLine { x: 16.5; y: 7 }
                PathLine { x: 12; y: 12 }
                PathLine { x: 16.5; y: 17 }
                PathLine { x: 16.5; y: 21 }
            }
        }
    }
}
