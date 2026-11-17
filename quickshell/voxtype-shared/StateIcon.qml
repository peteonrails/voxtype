// Vector state icon for the OSD: a microphone while recording, audio
// lines while streaming, and a spinning arc while transcribing. Setting
// `result` shows how a session ended instead: `done` (a check),
// `nospeech` (a crossed-out microphone) or `error` (a crossed circle).
//
// Drawn with QtQuick.Shapes instead of Nerd Font glyphs, so it renders
// the same without that font installed and stays crisp at any size. The
// paths are static; the spinner turns by `phase` (seconds), which the
// host advances on its own animation tick instead of a free-running
// animation that would redraw at the display's full refresh rate.
//
// Usage:
//
//   VT.StateIcon {
//       width: 24; height: 24
//       daemonState: "recording"
//       color: "#e06c75"
//       phase: renderer.phase
//   }

import QtQuick
import QtQuick.Shapes

Item {
    id: root

    property string daemonState: "recording"
    property color color: "#dcdfe4"
    /// Seconds of animation time; drives the transcribing spinner.
    property real phase: 0.0
    property real strokeWidth: 2.0
    /// Overrides the state icon when set: "done", "nospeech" or "error".
    property string result: ""

    implicitWidth: 24
    implicitHeight: 24

    readonly property string _kind: result === "done" || result === "error" ? result
        : result === "nospeech" ? "mic"
        : daemonState === "transcribing" ? "spinner"
        : daemonState === "streaming" ? "lines"
        : "mic"

    // Paths are authored on a 24x24 grid and scaled to the item.
    Item {
        width: 24
        height: 24
        anchors.centerIn: parent
        scale: Math.min(root.width, root.height) / 24

        Shape {
            anchors.fill: parent
            visible: root._kind === "mic"
            preferredRendererType: Shape.CurveRenderer
            ShapePath {
                strokeColor: root.color
                strokeWidth: root.strokeWidth
                fillColor: "transparent"
                capStyle: ShapePath.RoundCap
                joinStyle: ShapePath.RoundJoin
                // Capsule
                PathRectangle { x: 9; y: 2.5; width: 6; height: 11.5; radius: 3 }
            }
            ShapePath {
                strokeColor: root.color
                strokeWidth: root.strokeWidth
                fillColor: "transparent"
                capStyle: ShapePath.RoundCap
                joinStyle: ShapePath.RoundJoin
                // Holder arc and stand
                startX: 5.5; startY: 10.5
                PathLine { x: 5.5; y: 11.5 }
                PathArc { x: 18.5; y: 11.5; radiusX: 6.5; radiusY: 6.5; direction: PathArc.Counterclockwise }
                PathLine { x: 18.5; y: 10.5 }
                PathMove { x: 12; y: 18 }
                PathLine { x: 12; y: 21.5 }
            }
        }

        // Slash across the microphone for "no speech".
        Shape {
            anchors.fill: parent
            visible: root.result === "nospeech"
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
            visible: root._kind === "done"
            preferredRendererType: Shape.CurveRenderer
            ShapePath {
                strokeColor: root.color
                strokeWidth: root.strokeWidth
                fillColor: "transparent"
                capStyle: ShapePath.RoundCap
                joinStyle: ShapePath.RoundJoin
                startX: 4.5; startY: 12.5
                PathLine { x: 9.5; y: 17.5 }
                PathLine { x: 19.5; y: 6.5 }
            }
        }

        Shape {
            anchors.fill: parent
            visible: root._kind === "error"
            preferredRendererType: Shape.CurveRenderer
            ShapePath {
                strokeColor: root.color
                strokeWidth: root.strokeWidth
                fillColor: "transparent"
                capStyle: ShapePath.RoundCap
                PathAngleArc { centerX: 12; centerY: 12; radiusX: 9; radiusY: 9; startAngle: 0; sweepAngle: 360 }
                PathMove { x: 9; y: 9 }
                PathLine { x: 15; y: 15 }
                PathMove { x: 15; y: 9 }
                PathLine { x: 9; y: 15 }
            }
        }

        Shape {
            anchors.fill: parent
            visible: root._kind === "lines"
            preferredRendererType: Shape.CurveRenderer
            ShapePath {
                strokeColor: root.color
                strokeWidth: root.strokeWidth
                fillColor: "transparent"
                capStyle: ShapePath.RoundCap
                startX: 3; startY: 10.5
                PathLine { x: 3; y: 13.5 }
                PathMove { x: 7.5; y: 7 }
                PathLine { x: 7.5; y: 17 }
                PathMove { x: 12; y: 3.5 }
                PathLine { x: 12; y: 20.5 }
                PathMove { x: 16.5; y: 7.5 }
                PathLine { x: 16.5; y: 16.5 }
                PathMove { x: 21; y: 10.5 }
                PathLine { x: 21; y: 13.5 }
            }
        }

        Shape {
            anchors.fill: parent
            visible: root._kind === "spinner"
            preferredRendererType: Shape.CurveRenderer
            rotation: (root.phase * 300) % 360
            ShapePath {
                strokeColor: Qt.rgba(root.color.r, root.color.g, root.color.b, root.color.a * 0.25)
                strokeWidth: root.strokeWidth
                fillColor: "transparent"
                PathAngleArc { centerX: 12; centerY: 12; radiusX: 8.5; radiusY: 8.5; startAngle: 0; sweepAngle: 360 }
            }
            ShapePath {
                strokeColor: root.color
                strokeWidth: root.strokeWidth
                fillColor: "transparent"
                capStyle: ShapePath.RoundCap
                PathAngleArc { centerX: 12; centerY: 12; radiusX: 8.5; radiusY: 8.5; startAngle: -90; sweepAngle: 110 }
            }
        }
    }
}
