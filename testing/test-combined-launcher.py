#!/usr/bin/env python3
"""Run the bundled launcher through real isolated Quickshell IPC and config writes.

Use integrated-evidence/runtime.sh ui to deny production config/service access.
Only the disposable Panel copy gets read-only test aliases; its IPC is unchanged.
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import time
import tomllib

p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--bundle', type=Path, required=True)
p.add_argument('--evidence', type=Path, required=True)
a = p.parse_args()
bundle = a.bundle.resolve()
out = a.evidence.resolve()
out.mkdir(parents=True, exist_ok=True)
root = out / 'isolated-omarchy'
shell = root / 'shell'
shell.mkdir(parents=True, exist_ok=True)
for child in Path('/usr/share/omarchy/shell').iterdir():
    if child.is_dir() and child.name != 'plugin':
        dest = shell / child.name
        if not dest.exists():
            dest.symlink_to(child.resolve(), target_is_directory=True)
plugin = shell / 'plugin'
shutil.copytree(bundle / 'omarchy-plugin', plugin, dirs_exist_ok=True)
panel = plugin / 'Panel.qml'
panel.write_text(panel.read_text().replace('id: root', 'id: root\n  readonly property alias formUnderTest: form', 1))
config = out / 'config.toml'
config.write_text('# Isolated combined launcher verification\nengine = "whisper"\n[text]\nfilter_filler_words = false\ncollapse_restarts = false\nlard_disfluency = false\n[tray]\nenabled = false # preserve\n')
wrapper = out / 'voxtype-test'
wrapper.write_text('#!/usr/bin/python3\nimport os,sys\nos.execv(' + repr(str(bundle / 'voxtype')) + ', [' + repr(str(bundle / 'voxtype')) + ', "--config", ' + repr(str(config)) + '] + sys.argv[1:])\n')
wrapper.chmod(0o755)
(plugin / 'dev.json').write_text(json.dumps({'voxtypeBin': str(wrapper)}))
(shell / 'shell.qml').write_text('''import QtQuick
import Quickshell
import Quickshell.Io
import "plugin" as Plugin
Item {
 id: test
 Plugin.Panel {
  id: panel
  manifest: ({__sourceDir: "PLUGIN"})
  shell: test
 }
 function summon(id, payload) { panel.open(payload); return true }
 function hide(id) { panel.close(); return true }
 IpcHandler {
  target: "combined-test"
  function status(): string {
   var tray = null; var lard = null
   for (var i=0;i<panel.keys.length;i++) {
    if (panel.keys[i].key === "tray.enabled") tray = panel.keys[i].value
    if (panel.keys[i].key === "text.lard_disfluency") lard = panel.keys[i].value
   }
   return JSON.stringify({opened:panel.opened, section:panel.activeSection, sections:panel.sections, tray:tray, lard:lard})
  }
  function toggleTray(): string {
   panel.activeSection = "Status"
   Qt.callLater(function() {
    var form = panel.formUnderTest
    for (var i=0;i<form.visibleKeys.length;i++) if (form.visibleKeys[i].key === "tray.enabled") {
     form.highlightIndex = i + form.leadingRows
     form.activateHighlight(); return
    }
    console.error("TRAY_ROW_MISSING")
   })
   return "requested"
  }
  function quit(): string { Qt.callLater(Qt.quit); return "bye" }
 }
}
'''.replace('PLUGIN', str(plugin)))
env = dict(os.environ, OMARCHY_PATH=str(root))
log = (out / 'quickshell.log').open('w')
proc = subprocess.Popen(['qs', '-p', str(shell)], env=env, stdout=log, stderr=subprocess.STDOUT)
results = []
def ipc(target, method, *args):
    r = subprocess.run(['omarchy-shell', target, method, *args], env=env, text=True, capture_output=True, timeout=5)
    return r

def wait_for(predicate):
    r = None
    deadline = time.monotonic() + 20
    while time.monotonic() < deadline:
        assert proc.poll() is None, (out / 'quickshell.log').read_text()
        r = ipc('combined-test', 'status')
        if r.returncode == 0:
            try:
                state = json.loads(r.stdout)
                if predicate(state):
                    return state
            except json.JSONDecodeError:
                pass
        time.sleep(.1)
    raise AssertionError('real IPC state timed out: ' + repr(r))
try:
    initial = wait_for(lambda s: s['opened'] is False)
    for n in range(2):
        r = subprocess.run([str(bundle / 'voxtype-configure-launcher')], env=env, text=True, capture_output=True, timeout=8)
        results.append({'launcher_open': n, 'exit': r.returncode, 'stdout': r.stdout, 'stderr': r.stderr})
        assert r.returncode == 0, results[-1]
        state = wait_for(lambda s: s['opened'] and s['tray'] is False and s['lard'] is False)
        assert 'Text Cleanup' in state['sections'] and 'Status' in state['sections'], state
    r = ipc('io.voxtype.settings', 'section', 'Text Cleanup')
    assert r.returncode == 0 and r.stdout.strip() == 'ok', r
    state = wait_for(lambda s: s['section'] == 'Text Cleanup')
    results.append({'real_quickshell_state': state, 'config_wrapper': str(wrapper)})
    for expected in (True, False):
        r = ipc('combined-test', 'toggleTray')
        assert r.stdout.strip() == 'requested', r
        state = wait_for(lambda s: s['tray'] is expected)
        saved = tomllib.loads(config.read_text())
        assert saved['tray']['enabled'] is expected, saved
        assert saved['text'] == {'filter_filler_words': False, 'collapse_restarts': False, 'lard_disfluency': False}, saved
        assert '# preserve' in config.read_text()
        results.append({'persisted_tray': expected, 'qml_state': state})
finally:
    ipc('combined-test', 'quit')
    try:
        proc.wait(timeout=4)
    except subprocess.TimeoutExpired:
        proc.kill(); proc.wait()
    log.close()
assert proc.returncode == 0
text = (out / 'quickshell.log').read_text()
assert not any(s in text for s in ('TypeError:', 'ReferenceError:', 'TRAY_ROW_MISSING'))
(out / 'result.json').write_text(json.dumps(results, indent=2) + '\n')
print('COMBINED_LAUNCHER_QS_PASS: exact bundled companion -> actual omarchy-shell -> isolated actual Panel IPC; idempotent open, Text Cleanup/LARD schema, Status tray toggle/readback twice')
