// Static popup edge, including the active theme's Hyprland gradient stops.
// Unlike aura this border does not turn, glow, or react to voice activity.
import QtQuick
import QtQuick.Shapes

Shape {
    id: root
    property real cornerRadius: 0
    property real thickness: 2
    property var colors: []
    property real angle: 0
    property var _stops: []
    readonly property real inset: Math.min(thickness, width / 2, height / 2)
    readonly property real dx: Math.cos(angle * Math.PI / 180)
    readonly property real dy: -Math.sin(angle * Math.PI / 180)
    readonly property real span: Math.abs(width * dx) + Math.abs(height * dy)
    preferredRendererType: Shape.CurveRenderer
    visible: thickness > 0

    function _rebuildStops() {
        const previous = _stops;
        const next = [];
        for (let i = 0; i < colors.length; i++) {
            next.push(stopFactory.createObject(edgeGradient, {
                position: colors.length === 1 ? 0 : i / (colors.length - 1),
                color: colors[i]
            }));
        }
        edgeGradient.stops = next;
        _stops = next;
        for (let i = 0; i < previous.length; i++) previous[i].destroy();
    }
    onColorsChanged: _rebuildStops()
    Component.onCompleted: _rebuildStops()

    Component { id: stopFactory; GradientStop {} }

    ShapePath {
        strokeColor: "transparent"
        fillRule: ShapePath.OddEvenFill
        fillGradient: LinearGradient {
            id: edgeGradient
            x1: root.width / 2 - root.dx * root.span / 2
            y1: root.height / 2 - root.dy * root.span / 2
            x2: root.width / 2 + root.dx * root.span / 2
            y2: root.height / 2 + root.dy * root.span / 2
        }
        PathRectangle {
            width: root.width; height: root.height
            radius: Math.min(root.cornerRadius, root.width / 2, root.height / 2)
        }
        PathRectangle {
            x: root.inset; y: root.inset
            width: Math.max(0, root.width - 2 * root.inset)
            height: Math.max(0, root.height - 2 * root.inset)
            radius: Math.max(0, Math.min(root.cornerRadius, root.width / 2, root.height / 2) - root.inset)
        }
    }
}
