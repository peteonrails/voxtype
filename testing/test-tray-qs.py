#!/usr/bin/env python3
"""Real Quickshell icon provider plus exact Omarchy onlyMenu click boundary.

A recording-free probe runs on an isolated session bus. No installed file changes,
recording daemon, shell restart, or production menu actions. Render captures are
real QQuick Window content under the offscreen software platform.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import time

p = argparse.ArgumentParser()
p.add_argument('--probe', required=True, type=Path)
p.add_argument('--evidence', required=True, type=Path)
p.add_argument('--worker', action='store_true')
p.add_argument('--live-host', action='store_true', help='Recording-free probe on current session bus, filtered by embedded pixmap provider')
a = p.parse_args()
a.probe = a.probe.resolve()
a.evidence = a.evidence.resolve()
a.evidence.mkdir(parents=True, exist_ok=True)
for state in ['idle', 'recording', 'transcribing', 'error']:
    (a.evidence/(state+'.png')).unlink(missing_ok=True)
if not a.worker and not a.live_host:
    raise SystemExit(subprocess.call(['dbus-run-session', '--', sys.executable, __file__, '--probe', str(a.probe), '--evidence', str(a.evidence), '--worker']))

host = Path('/usr/share/omarchy/shell/plugins/bar/widgets/Tray.qml')
source = host.read_text()
def body(marker):
    start = source.index(marker) + len(marker)
    level = 1
    end = start
    while level:
        if source[end] == '{': level += 1
        elif source[end] == '}': level -= 1
        end += 1
    return source[start:end-1]
click = source[source.index('      onClicked: function(mouse) {', source.index('  component TrayItem: Item {')):]
click = click[click.index('{')+1:click.index('\n      }')]
open_menu = body('function openTrayMenu(item, anchorItem, mouse) {')
(a.evidence/'host-click.txt').write_text(click)
(a.evidence/'host-open-menu.txt').write_text(open_menu)
qml = '''import QtQuick
import QtQuick.Window
import Quickshell
import Quickshell.Widgets
import Quickshell.Services.SystemTray
Window {
 id: root
 visible: true; width: 440; height: 160; color: "#181818"
 property var activeTrayItem: null
 property var activeTrayAnchor: null
 property bool trayMenuOpen: false
 property bool managePopupOpen: false
 property bool barDragging: false
 property var bar: null
 property var item: null
 property bool clicked: false
 property bool selected: false
 property int frames: 0
 property string state: ""
 readonly property bool holdOpen: trayMenuOpen || managePopupOpen || barDragging || (!!bar && bar.traySectionPopoutOpen === true)
 function resetTrayMenu() {}
 function openTrayMenu(item, anchorItem, mouse) { OPEN_MENU }
 QtObject {
  id: trayItemRoot
  property var modelData: root.item
  function displayMenu(mouse) { root.openTrayMenu(modelData, scene, mouse) }
 }
 function click(mouse) { CLICK_BODY }
 QsMenuOpener { id: opener; menu: root.item ? root.item.menu : null }
 Item {
  id: scene; anchors.fill: parent
  Text { x: 16; y: 12; text: "VoxType native tray / " + root.state; color: "white"; font.pixelSize: 18 }
  Rectangle { x: 20; y: 52; width: 64; height: 64; color: "#202020" }
  IconImage { id: icon; x: 20; y: 52; width: 64; height: 64; source: root.item ? root.item.icon : "" }
  Text { x: 110; y: 62; text: "embedded icon, missing hicolor theme\nonlyMenu: " + (root.item ? root.item.onlyMenu : false); color: "white"; font.pixelSize: 14 }
 }
 Timer {
  interval: 120; running: true; repeat: true
  onTriggered: {
   var values = SystemTray.items.values
   for (var i = 0; i < values.length; i++) if (values[i].id === "voxtype" && (!LIVE_FILTER || String(values[i].icon).indexOf("image://qspixmap/") === 0)) root.item = values[i]
   if (!root.item) return
   if (!root.clicked) {
    root.clicked = true
    root.click({button: Qt.LeftButton, accepted: false, x: 0, y: 0})
    console.log("CLICK_BOUNDARY " + JSON.stringify({onlyMenu: root.item.onlyMenu, trayMenuOpen: root.trayMenuOpen, holdOpen: root.holdOpen, icon: String(root.item.icon)}))
   }
   var label = root.item.tooltipTitle
   var next = label.indexOf("Recording") >= 0 ? "recording" : label.indexOf("Transcribing") >= 0 ? "transcribing" : label.indexOf("failed") >= 0 ? "error" : "idle"
   if (next !== root.state) { root.state = next; root.frames = 0 }
   if (++root.frames === 5) scene.grabToImage(function(result) { console.log("CAPTURE " + root.state + " " + result.saveToFile(OUT + "/" + root.state + ".png")) })
   var rows = opener.children.values
   if (root.state === "error" && root.frames === 8 && !root.selected) {
    for (var j = 0; j < rows.length; j++) if (rows[j].text === "Settings") {
     root.selected = true; console.log("SETTINGS_SELECTION"); rows[j].triggered()
    }
   }
  }
 }
}
'''.replace('OPEN_MENU', open_menu).replace('CLICK_BODY', click).replace('OUT', json.dumps(str(a.evidence))).replace('LIVE_FILTER', str(a.live_host).lower())
(a.evidence/'shell.qml').write_text(qml)
qlog = (a.evidence/'quickshell.log').open('w')
qs = subprocess.Popen(['qs', '-p', str(a.evidence/'shell.qml')], stdout=qlog, stderr=subprocess.STDOUT, env=dict(os.environ, QT_QPA_PLATFORM='offscreen', QT_QUICK_BACKEND='software'))
probe = None
try:
    time.sleep(.7)
    plog = (a.evidence/'probe.log').open('w')
    probe = subprocess.Popen([str(a.probe)], stdin=subprocess.PIPE, stdout=plog, stderr=subprocess.STDOUT, text=True)
    assert probe.stdin is not None
    for state in ['idle', 'recording', 'transcribing', 'error']:
        probe.stdin.write(state+'\n'); probe.stdin.flush()
        time.sleep(1.7)
        assert qs.poll() is None, (a.evidence/'quickshell.log').read_text()
        assert (a.evidence/(state+'.png')).is_file(), (state, (a.evidence/'quickshell.log').read_text())
        if state != 'error': assert 'ACTION OpenSettings' not in (a.evidence/'probe.log').read_text(), 'icon click launched settings'
    logs = (a.evidence/'quickshell.log').read_text()
    boundary = json.loads(logs.split('CLICK_BOUNDARY ')[1].splitlines()[0])
    assert boundary['onlyMenu'] and boundary['trayMenuOpen'] and boundary['holdOpen'], boundary
    assert 'SETTINGS_SELECTION' in logs and 'ACTION OpenSettings' in (a.evidence/'probe.log').read_text()
    assert 'Failed to get image' not in logs and 'Cannot open' not in logs, logs
    (a.evidence/'result.json').write_text(json.dumps({'passed': True, 'host_sha256': hashlib.sha256(host.read_bytes()).hexdigest(), 'probe_sha256': hashlib.sha256(a.probe.read_bytes()).hexdigest(), 'boundary': boundary, 'settings_only_explicit': True, 'live_host_bus': a.live_host, 'states': ['idle','recording','transcribing','error'], 'scope': 'real QS provider/render, exact installed host click/open functions, isolated bus and recording-free probe; not a production drawer click'}, indent=2)+'\n')
    print('QS_TRAY_PASS: real rendered pixmaps, all four states, exact host onlyMenu branch holds drawer; settings explicit only')
finally:
    if probe:
        if probe.poll() is None:
            probe.stdin.write('quit\n'); probe.stdin.flush(); probe.wait(timeout=5)
        plog.close()
    qs.terminate(); qs.wait(timeout=5); qlog.close()
