#!/usr/bin/python
"""Render the production license Text with Qt Quick (system PySide6 required).

/usr/bin/python omarchy-plugin/tests/test-license-link-contrast.py --evidence DIR
The minimal Item host substitutes only shell tokens and the external URL sink;
production text, format, wrapping and activation handler are extracted unchanged.
"""
import argparse
from collections import Counter
import json
import os
from pathlib import Path
import re
import sys

os.environ.setdefault('QT_QPA_PLATFORM', 'offscreen')
os.environ.setdefault('QT_QUICK_BACKEND', 'software')
from PySide6.QtCore import QMetaObject, QPoint, QUrl, Qt, Q_RETURN_ARG, qInstallMessageHandler
from PySide6.QtGui import QColor, QGuiApplication
from PySide6.QtQml import QQmlComponent, QQmlEngine
from PySide6.QtQuick import QQuickWindow
from PySide6.QtTest import QTest

parser = argparse.ArgumentParser()
parser.add_argument('--source', type=Path, default=Path(__file__).resolve().parents[1] / 'components/SchemaForm.qml')
parser.add_argument('--evidence', type=Path, required=True)
parser.add_argument('--control', type=Path)
args = parser.parse_args()
args.evidence.mkdir(parents=True, exist_ok=True)
source = args.source.read_text()
match = re.search(r'Text \{\s*objectName: "lardModelLicense".*?\n            \}', source, re.S)
assert match, 'Production license Text not found'
block = match.group()
assert 'onLinkActivated: function(link) { Qt.openUrlExternally(link) }' in block
url = re.search(r'readonly property string lardLicenseUrl: ("[^"]+")', source).group(1)
# Capture the real handler route, without letting a test open a web browser.
block = block.replace('Qt.openUrlExternally(link)', 'root.activatedUrl = link')
block = block.replace('ShellTheme.Color.foreground', 'root.foreground')
block = block.replace('Style.font.family', '"monospace"').replace('Style.font.caption', '14')
block = block.replace('Text {', 'Text { id: license;', 1)
qml = '''import QtQuick
Rectangle {
 id: root
 width: 780; height: 300
 property color foreground: "#e6d9db"
 property string activatedUrl: ""
 readonly property string lardLicenseUrl: URL
 function cleanupHeading(index) { return "LARD disfluency cleanup" }
 QtObject { id: entry; property int index: 0 }
 color: "#2c2525"
 BLOCK
 function hitPoints() {
   var hits = []
   for (var y = 0; y < license.height; y++)
     for (var x = 0; x < license.width; x++)
       if (license.linkAt(x, y) === lardLicenseUrl) hits.push([x,y])
   return JSON.stringify(hits)
 }
 function metrics() { return JSON.stringify([license.width, license.height, license.contentWidth, license.contentHeight]) }
}'''.replace('URL', url).replace('BLOCK', block)
(args.evidence / 'render-host.qml').write_text(qml)
warnings = []
qInstallMessageHandler(lambda kind, context, message: warnings.append(message))
app = QGuiApplication([])
engine = QQmlEngine()
component = QQmlComponent(engine)
component.setData(qml.encode(), QUrl())
root = component.create()
assert root, [e.toString() for e in component.errors()]
window = QQuickWindow()
root.setParentItem(window.contentItem())
window.resize(780, 300)
window.show()
results = []
failures = []

def luminance(rgb):
    values = [v / 255 for v in rgb]
    values = [v / 12.92 if v <= 0.04045 else ((v + 0.055) / 1.055) ** 2.4 for v in values]
    return sum(v * w for v, w in zip(values, (0.2126, 0.7152, 0.0722)))

for theme, fg, bg in [('dark', '#e6d9db', '#2c2525'), ('light', '#242424', '#fafafa')]:
    root.setProperty('foreground', QColor(fg))
    root.setProperty('color', QColor(bg))
    for width in (360, 780):
        root.setWidth(width)
        window.resize(width, 300)
        QTest.qWait(100)
        points = json.loads(QMetaObject.invokeMethod(root, 'hitPoints', Q_RETURN_ARG('QVariant')))
        assert points, 'No clickable license anchor'
        metrics = json.loads(QMetaObject.invokeMethod(root, 'metrics', Q_RETURN_ARG('QVariant')))
        image = window.grabWindow()
        assert not image.isNull(), 'No actual rendered capture'
        image.save(str(args.evidence / f'{theme}-{width}.png'))
        background = QColor(bg).rgb() & 0xffffff
        scale = image.devicePixelRatio()
        physical_points = {(int(x * scale), int(y * scale)) for x, y in points}
        pixels = Counter(image.pixel(x, y) & 0xffffff for x, y in physical_points if image.pixel(x, y) & 0xffffff != background)
        dominant, count = pixels.most_common(1)[0]
        rgb = ((dominant >> 16) & 255, (dominant >> 8) & 255, dominant & 255)
        bg_rgb = (QColor(bg).red(), QColor(bg).green(), QColor(bg).blue())
        a, b = sorted((luminance(rgb), luminance(bg_rgb)))
        contrast = (b + 0.05) / (a + 0.05)
        expected = QColor(fg).rgb() & 0xffffff
        if dominant != expected or count < 30 or contrast < 4.5:
            failures.append(f'{theme}/{width}: rendered #{dominant:06x}, expected {fg}, contrast {contrast:.3f}:1')
        # A real mouse click must traverse Text.linkActivated and its original URL route.
        root.setProperty('activatedUrl', '')
        x, y = points[len(points)//2]
        QTest.mouseClick(window, Qt.LeftButton, Qt.NoModifier, QPoint(x,y))
        assert root.property('activatedUrl') == json.loads(url), 'Link click route changed'
        results.append(dict(theme=theme, width=width, rendered=f'#{dominant:06x}', solid_pixels=count, contrast=contrast, metrics=metrics, click_url=root.property('activatedUrl')))

if args.control:
    control = json.loads(args.control.read_text())['results']
    assert [r['metrics'] for r in results] == [r['metrics'] for r in control], 'Wrapping/layout changed'
assert not warnings, warnings
report = dict(source=str(args.source), results=results, warnings=warnings, failures=failures)
(args.evidence / 'results.json').write_text(json.dumps(report, indent=2))
print(json.dumps(report, indent=2))
window.close()
root.setParentItem(None)
del root, component, engine
sys.exit(bool(failures))
