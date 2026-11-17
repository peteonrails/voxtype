#!/usr/bin/env python3
"""Actual QS provider/menu + installed Omarchy click/lifetime functions.

--host-patch is an isolated proposal only; never modifies the installed host.
No recording actions execute, and the only writable config is this fixture.
"""
import argparse
import difflib
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import time

parser = argparse.ArgumentParser()
parser.add_argument('--probe', type=Path, required=True)
parser.add_argument('--evidence', type=Path, required=True)
parser.add_argument('--worker', action='store_true')
parser.add_argument('--host-patch', action='store_true')
a = parser.parse_args()
a.evidence = a.evidence.resolve()
a.probe = a.probe.resolve()
a.evidence.mkdir(parents=True, exist_ok=True)
if not a.worker:
    cmd = ['dbus-run-session', '--', sys.executable, __file__, '--probe', str(a.probe), '--evidence', str(a.evidence), '--worker']
    if a.host_patch:
        cmd.append('--host-patch')
    raise SystemExit(subprocess.call(cmd))
host = Path('/usr/share/omarchy/shell/plugins/bar/widgets/Tray.qml')
original = host.read_text()
source = original
if a.host_patch:
    marker = '  function openTrayMenu(item, anchorItem, mouse) {\n'
    source = source.replace(marker, marker + '''    if (trayMenuOpen && activeTrayItem === item) {
      close()
      return
    }
''', 1)
    assert source != original
    (a.evidence/'omarchy-tray-toggle.patch').write_text(''.join(difflib.unified_diff(original.splitlines(True), source.splitlines(True), fromfile='a/shell/plugins/bar/widgets/Tray.qml', tofile='b/shell/plugins/bar/widgets/Tray.qml')))

def body(marker):
    start = source.index(marker) + len(marker)
    level, end = 1, start
    while level:
        if source[end] == '{':
            level += 1
        elif source[end] == '}':
            level -= 1
        end += 1
    return source[start:end-1]

click = body('      onClicked: function(mouse) {')
# The only onClicked: function(mouse) is the tray-icon handler.
functions = '\n'.join('function ' + signature + ' {' + body('function ' + signature + ' {') + '}' for signature in [
    'openTrayMenu(item, anchorItem, mouse)', 'close()', 'resetTrayMenu()',
    'enterSubmenu(entry, title)', 'leaveSubmenu()', 'settleMenuLevel()'])
config = a.evidence/'config.toml'
config.write_text('''# isolated language fixture
engine = "cohere"
[cohere]
model = "cohere-transcribe-03-2026-Q4_K_M.gguf"
language = "en" # preserve language comment
[whisper]
language = "de"
''')
qml = '''import QtQuick
import QtQuick.Window
import Quickshell
import Quickshell.Services.SystemTray
Window {
 id: root
 visible: true; width: 680; height: 650; color: "#181818"
 property var item: null
 property var activeTrayItem: null
 property var activeTrayAnchor: null
 property bool trayMenuOpen: false
 property bool managePopupOpen: false
 property bool expanded: true
 property bool barDragging: false
 property var bar: null
 property var submenuStack: []
 property bool menuLevelSettling: false
 property int phase: 0
 property int ticks: 0
 readonly property bool holdOpen: trayMenuOpen || managePopupOpen || barDragging || (!!bar && bar.traySectionPopoutOpen === true)
 readonly property int submenuDepth: submenuStack.length
 readonly property var currentChildren: submenuDepth > 0 ? submenuStack[submenuDepth - 1].opener.children : trayMenuOpener.children
 Component { id: submenuOpenerComponent; QsMenuOpener {} }
 QtObject { id: trayMenuFlick; property real contentY: 0 }
 Timer { id: menuLevelSettleTimer; interval: 250; onTriggered: root.menuLevelSettling = false }
 Timer { id: fade; interval: 150; onTriggered: if (!root.trayMenuOpen) root.resetTrayMenu() }
 onTrayMenuOpenChanged: if (!trayMenuOpen) fade.restart()
 __FUNCTIONS__
 QtObject {
  id: trayItemRoot
  property var modelData: root.item
  function displayMenu(mouse) { root.openTrayMenu(modelData, scene, mouse) }
 }
 function click(mouse) { __CLICK__ }
 function iconClick() { click({button: Qt.LeftButton,accepted:false,x:0,y:0}) }
 function assertState(condition, label) {
  console.log("ASSERT " + label + " " + condition)
  if (!condition) { console.log("FAIL " + label); Qt.quit() }
 }
 function rows() { return root.currentChildren.values }
 function find(prefix) {
  var list=rows()
  for (var i=0;i<list.length;i++) if (list[i].text.indexOf(prefix)===0) return list[i]
  return null
 }
 QsMenuOpener { id: trayMenuOpener; menu: root.activeTrayItem ? root.activeTrayItem.menu : null }
 Item {
  id: scene; anchors.fill: parent
  Text { x:16; y:12; color:"white"; font.pixelSize:18; text:"VoxType Language / real QS menu" }
  Column {
   x:16; y:50; spacing:5
   Repeater {
    model: root.currentChildren
    Text { required property var modelData; color: modelData.enabled ? "white":"#a0a0a0"; font.pixelSize:17
     text:(modelData.checkState===Qt.Checked ? "✓ ":"   ")+modelData.text+(modelData.hasChildren ? " ›":"") }
   }
  }
 }
 Timer {
  interval:160; repeat:true; running:true
  onTriggered: {
   ticks++
   var items=SystemTray.items.values
   for(var i=0;i<items.length;i++) if(items[i].id==="voxtype") root.item=items[i]
   if (!root.item || ticks<5) return
   if (phase===0) {
    assertState(root.item.onlyMenu,"native-only-menu")
    iconClick(); assertState(trayMenuOpen && holdOpen && expanded,"first-click-open")
    phase=1; ticks=0
   } else if (phase===1 && ticks>3) {
    iconClick(); assertState(!trayMenuOpen && expanded,"second-click-closes-without-drawer-collapse")
    phase=2;ticks=0
   } else if (phase===2 && ticks>3) {
    iconClick(); assertState(trayMenuOpen && holdOpen && submenuDepth===0,"fresh-reopen-root")
    phase=3;ticks=0
   } else if (phase===3 && ticks>3) {
    var language=find("Language:")
    assertState(language && language.hasChildren,"language-submenu-real-entry")
    enterSubmenu(language,language.text);phase=4;ticks=0
   } else if (phase===4 && ticks>4) {
    assertState(submenuDepth===1 && rows().length===16,"nested-fourteen-radios")
    assertState(rows()[0].text.indexOf("Q4_K_M.gguf")>=0,"literal-model-name-preserved")
    var english=find("english (en)")
    assertState(english && english.checkState===Qt.Checked,"english-current-checked")
    phase=104
    scene.grabToImage(function(r){
     r.saveToFile(__OUTPUT__+"/language-en.png")
     iconClick();assertState(!trayMenuOpen && expanded,"nested-second-click-closes")
     phase=5;ticks=0
    })
   } else if (phase===5 && ticks>3) {
    iconClick();assertState(trayMenuOpen && submenuDepth===0,"nested-close-fresh-root-reopen")
    phase=6;ticks=0
   } else if (phase===6 && ticks>3) {
    var language=find("Language:");enterSubmenu(language,language.text);phase=7;ticks=0
   } else if (phase===7 && ticks>4) {
    var french=find("french (fr)");assertState(french && french.enabled,"french-selectable")
    french.triggered();root.close();phase=8;ticks=0
   } else if (phase===8 && ticks>5) {
    iconClick();phase=9;ticks=0
   } else if (phase===9 && ticks>3) {
    var language=find("Language: fr");assertState(!!language,"readback-updated-label")
    enterSubmenu(language,language.text);phase=10;ticks=0
   } else if (phase===10 && ticks>4) {
    assertState(find("french (fr)").checkState===Qt.Checked,"french-success-checked")
    phase=110
    scene.grabToImage(function(r){
     r.saveToFile(__OUTPUT__+"/language-fr.png")
     leaveSubmenu();phase=11;ticks=0
    })
   } else if (phase===11 && ticks>4) {
    var settings=find("Settings");assertState(!!settings,"settings-explicit-entry")
    console.log("EXPLICIT_SETTINGS");settings.triggered();console.log("PASS_LANGUAGE_HOST");Qt.quit()
   }
  }
 }
}
'''.replace('__FUNCTIONS__', functions).replace('__CLICK__', click).replace('__OUTPUT__', json.dumps(str(a.evidence)))
(a.evidence/'shell.qml').write_text(qml)
qlog = (a.evidence/'qs.log').open('w')
plog = (a.evidence/'probe.log').open('w')
qs = subprocess.Popen(['qs','-p',str(a.evidence/'shell.qml')], stdout=qlog, stderr=subprocess.STDOUT, env=dict(os.environ, QT_QPA_PLATFORM='offscreen', QT_QUICK_BACKEND='software'))
probe = subprocess.Popen([str(a.probe),str(config)], stdin=subprocess.PIPE, stdout=plog, stderr=subprocess.STDOUT, text=True)
try:
    qs.wait(timeout=30)
    logs = (a.evidence/'qs.log').read_text()
    assert 'PASS_LANGUAGE_HOST' in logs and 'FAIL ' not in logs, logs
    time.sleep(.3)
    actions = (a.evidence/'probe.log').read_text()
    assert actions.count('ACTION OpenSettings') == 1, actions
    assert 'ToggleRecording' not in actions and 'LANGUAGE_SAVED fr' in actions, actions
    persisted = config.read_text()
    assert 'language = "fr" # preserve language comment' in persisted and 'language = "de"' in persisted
    assert host.read_text() == original
    result = {'passed':True,'installed_host_sha256':hashlib.sha256(original.encode()).hexdigest(),'host_patch_required':a.host_patch,'production_host_modified':False,'probe_sha256':hashlib.sha256(a.probe.read_bytes()).hexdigest(),'scope':'Actual QS/menu and exact installed host functions in isolated Window fixture; hover/drawer geometry and physical production clicks not claimed'}
    (a.evidence/'result.json').write_text(json.dumps(result,indent=2)+'\n')
    print('LANGUAGE_QS_HOST_PASS')
finally:
    if qs.poll() is None:
        qs.terminate(); qs.wait(timeout=5)
    if probe.poll() is None:
        probe.stdin.write('quit\n');probe.stdin.flush();probe.wait(timeout=5)
    qlog.close();plog.close()
