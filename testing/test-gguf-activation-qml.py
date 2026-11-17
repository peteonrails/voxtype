#!/usr/bin/env python3
"""Real RC2 Settings browsing regression. Requires reviewed runtime-isolate.py.

Uses actual Panel/SchemaForm/EnginePicker/VoxtypeCli with test-only aliases,
QtTest pointer/keyboard events and a private headless labwc. No downloads.
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
p.add_argument('--plugin', type=Path, required=True)
p.add_argument('--out', type=Path, required=True)
p.add_argument('--installed-target', action='store_true')
p.add_argument('--active-use', action='store_true')
p.add_argument('--fail-use', action='store_true')
p.add_argument('--nemotron-env', action='store_true')
a = p.parse_args()
if (os.environ.get('RC2_DESKTOP_ISOLATED') != '1'
        or Path('/proc/1/comm').read_text().strip() != 'bwrap'
        or Path('/run/user').exists() or Path('/dev/input').exists()
        or Path('/dev/uinput').exists()):
    p.error('Reviewed private runtime required; host execution forbidden')
a.out.mkdir(parents=True)
root = a.out / 'fixture'
root.mkdir()
home = root / 'home'
runtime = root / 'runtime'
home.mkdir()
runtime.mkdir(mode=0o700)
config = home / '.config/voxtype/config.toml'
config.parent.mkdir(parents=True)
original = ('# preserve working Ultra F16 and comments\nengine = "parakeet"\n'
            '[parakeet]\nmodel = "parakeet-ultra-F16.gguf" # keep\n'
            '[gguf]\nmodel = "Qwen3-ASR-0.6B-Q8_0.gguf"\n'
            'architecture = "qwen3_asr"\nlanguage = "de"\n')
if a.nemotron_env:
    original = original.replace('language = "de"', 'language = "en-US"')
target_model = 'nemotron-3.5-asr-streaming-0.6b-Q8_0.gguf' if a.nemotron_env else 'Qwen3-ASR-0.6B-Q8_0.gguf'
config.write_text(original)
store = home / '.local/share/voxtype/models'
store.mkdir(parents=True)
(store / 'parakeet-ultra-F16.gguf').symlink_to('/models/parakeet-ultra-F16.gguf')
if a.active_use:
    (store / 'parakeet-tdt-0.6b-v3-F16.gguf').symlink_to('/models/parakeet-tdt-0.6b-v3-F16.gguf')
if a.installed_target:
    (store / target_model).symlink_to('/models/' + target_model)
env = dict(os.environ, HOME=str(home), XDG_RUNTIME_DIR=str(runtime),
           XDG_CONFIG_HOME=str(home / '.config'), XDG_DATA_HOME=str(home / '.local/share'),
           XDG_CACHE_HOME=str(home / '.cache'), WLR_BACKENDS='headless',
           WLR_RENDERER='pixman', QT_QPA_PLATFORM='wayland', QT_QUICK_BACKEND='software',
           LD_LIBRARY_PATH='/bundle/tools/lib',
           VOXTYPE_GGUF_WORKER='/bundle/voxtype-gguf-worker-vulkan',
           DBUS_SESSION_BUS_ADDRESS='unix:path=' + str(runtime / 'absent-bus'))
env.pop('WAYLAND_DISPLAY', None)
if a.nemotron_env:
    env['VOXTYPE_GGUF_LANGUAGE'] = 'de'
# Real backend command capture, not a backend response mock.
wrapper = root / 'voxtype-test'
commands = a.out / 'commands.jsonl'
wrapper.write_text('#!/usr/bin/python3\nimport json,os,sys\n'
                   'with open(' + repr(str(commands)) + ',"a") as f: f.write(json.dumps(sys.argv[1:])+"\\n")\n'
                   + ('if sys.argv[1:3] == ["config","select-model"]: os.environ["VOXTYPE_GGUF_WORKER"]="/missing-worker"\n' if a.fail_use else '')
                   + 'os.execv("/bundle/voxtype-vulkan",["/bundle/voxtype-vulkan","--config",'
                   + repr(str(config)) + ']+sys.argv[1:])\n')
wrapper.chmod(0o755)
for directory in Path('/usr/share/omarchy/shell').iterdir():
    if directory.is_dir():
        (root / directory.name).symlink_to(directory)
plugin = root / 'plugin'
shutil.copytree(a.plugin, plugin)
(plugin / 'dev.json').write_text(json.dumps({'voxtypeBin': str(wrapper)}))
def expose(path, old, new):
    file = plugin / path
    text = file.read_text()
    assert old in text, (path, old)
    file.write_text(text.replace(old, new, 1))
expose('Panel.qml', 'id: root', 'id: root\n  readonly property alias formTest: form\n  readonly property alias cliTest: cli\n  readonly property alias renderTest: settingsCard.card')
expose('components/SchemaForm.qml', 'id: root', 'id: root\n  readonly property alias pickerTest: pickerProbe\n  readonly property alias cardTest: cardProbe')
expose('components/SchemaForm.qml', 'EnginePicker {', 'EnginePicker {\n      id: pickerProbe')
expose('components/SchemaForm.qml', 'EngineModelCard {', 'EngineModelCard {\n          id: cardProbe')
expose('components/EnginePicker.qml', 'id: root', 'id: root\n  readonly property alias dropdownTest: engineDropdown')
# Inspect actual rendered buttons, not just the catalog's data model.
qml = '''import QtQuick
import QtTest
import Quickshell
import "plugin" as Plugin
Item {
 id: host
 Plugin.Panel { id: panel; manifest: ({__sourceDir: PLUGIN}) }
 TestCase {
  id: tc
  parent: panel.renderTest
  name: "GGUFEmptyModelBrowse"
  when: false
  property bool started: false
  Timer { interval: 100; running: !tc.started; repeat: true; onTriggered: {
   if (panel.opened && panel.engine === "parakeet" && panel.formTest.cardTest.modelRows.length > 0) {
    tc.started = true
    tc.test_browse()
   }
  } }
  function texts(item, wanted) {
   var found = []
   if (item.text !== undefined && String(item.text) === wanted && item.visible) found.push(item)
   if (item.children) for (var i = 0; i < item.children.length; i++) found = found.concat(texts(item.children[i], wanted))
   return found
  }
  function buttons(item, wanted) {
   return texts(item, wanted).filter(function(item) { return item.tooltipText !== undefined })
  }
  function check(actual, expected, message) {
   if (actual !== expected) {
    console.log("MINIMAL_BROWSE_FAIL: " + (message || "") + " actual=" + actual + " expected=" + expected)
    throw new Error("browser assertion")
   }
  }
  function test_browse() {
   var picker = panel.formTest.pickerTest
   var dropdown = picker.dropdownTest
   mouseClick(dropdown, dropdown.width / 2, dropdown.height / 2)
   tryCompare(dropdown, "popupOpen", true)
   var from = -1, target = -1
   for (var i = 0; i < dropdown.options.length; i++) {
    if (dropdown.options[i].value === "parakeet") from = i
    if (dropdown.options[i].value === "gguf") target = i
   }
   verify(from >= 0 && target >= 0, "real engine choices missing")
   for (var j = from; j < target; j++) keyClick(Qt.Key_Down)
   for (var k = from; k > target; k--) keyClick(Qt.Key_Up)
   keyClick(Qt.Key_Return)
   tryCompare(dropdown, "popupOpen", false)
   wait(2000) // let an erroneous RC2 config write finish, retain exact error
   console.log("BROWSE_STATE " + JSON.stringify({picker: dropdown.value, active: panel.engine, catalog: panel.formTest.cardTest.engine, error: panel.engineError}))
   panel.renderTest.grabToImage(function(result) { result.saveToFile(CAPTURE) })
   wait(300)
   check(dropdown.value, "gguf", "GGUF picker must not snap back: " + panel.engineError)
   check(panel.formTest.cardTest.engine, "gguf")
   check(panel.engine, "parakeet", "browse must preserve configured engine")
   check(panel.modelValue, "parakeet-ultra-F16.gguf")
   check(panel.engineError, "", "no validation wall needed to browse")
   var card = panel.formTest.cardTest
   check(card.modelRows.length, 5)
   check(buttons(card, "Download").length, INSTALLED ? 4 : 5)
   check(buttons(card, "Use").length, INSTALLED ? 1 : 0)
   verify(texts(panel.renderTest, "Configured: parakeet").length === 1, "configured selection must be explicit")
   // A schema refresh must not discard browse intent.
   panel.cliTest.fetchSchema()
   wait(1000)
   check(dropdown.value, "gguf")
   check(card.engine, "gguf")
   if (INSTALLED) {
    var targetUse = buttons(card, "Use")[0]
    mouseClick(targetUse, targetUse.width / 2, targetUse.height / 2)
    if (FAIL_USE) {
     tryVerify(function() { return panel.engineError !== "" }, 4000)
     check(panel.engine, "parakeet")
     check(card.engine, "gguf")
     check(dropdown.value, "gguf")
    } else {
     tryCompare(panel, "engine", "gguf", 4000)
     check(panel.modelValue, "Qwen3-ASR-0.6B-Q8_0.gguf")
     check(panel.engineError, "")
    }
    console.log("ATOMIC_USE_PASS")
   }
   if (ACTIVE_USE) {
    mouseClick(dropdown, dropdown.width / 2, dropdown.height / 2)
    tryCompare(dropdown, "popupOpen", true)
    for (var n = target; n > from; n--) keyClick(Qt.Key_Up)
    for (var m = target; m < from; m++) keyClick(Qt.Key_Down)
    keyClick(Qt.Key_Return)
    wait(200)
    var use = buttons(card, "Use")
    check(use.length, 1, "installed alternative for active engine")
    mouseClick(use[0], use[0].width / 2, use[0].height / 2)
    tryCompare(panel, "modelValue", "parakeet-tdt-0.6b-v3-F16.gguf")
    check(panel.engine, "parakeet")
    check(panel.engineError, "")
    console.log("ACTIVE_USE_PASS")
   }
   console.log("MINIMAL_BROWSE_PASS")
   Qt.callLater(Qt.quit)
  }
 }
 Component.onCompleted: Qt.callLater(function() { panel.open('{"section":"Engine"}') })
 Timer { interval: 18000; running: true; onTriggered: Qt.quit() }
}
'''.replace('PLUGIN', json.dumps(str(plugin))).replace('CAPTURE', json.dumps(str(a.out / 'catalog.png'))).replace('INSTALLED', 'true' if a.installed_target else 'false').replace('ACTIVE_USE', 'true' if a.active_use else 'false').replace('FAIL_USE', 'true' if a.fail_use else 'false')
qml = qml.replace('Qwen3-ASR-0.6B-Q8_0.gguf', target_model)
(root / 'shell.qml').write_text(qml)
with (a.out / 'compositor.log').open('w') as log:
    compositor = subprocess.Popen(['/bundle/tools/bin/labwc', '-C', str(root / 'empty-labwc')], env=env, stdout=log, stderr=log, start_new_session=True)
    try:
        sockets = []
        for _ in range(100):
            sockets = [x for x in runtime.glob('wayland-*') if not x.name.endswith('.lock')]
            if sockets or compositor.poll() is not None:
                break
            time.sleep(0.1)
        assert sockets and compositor.poll() is None, 'private compositor failed'
        env['WAYLAND_DISPLAY'] = sockets[0].name
        result = subprocess.run(['qs', '-p', str(root / 'shell.qml')], env=env, capture_output=True, text=True, timeout=25)
        (a.out / 'stdout').write_text(result.stdout)
        (a.out / 'stderr').write_text(result.stderr)
        (a.out / 'exit').write_text(str(result.returncode) + '\n')
        (a.out / 'config.toml').write_bytes(config.read_bytes())
        expected = original.replace('parakeet-ultra-F16.gguf', 'parakeet-tdt-0.6b-v3-F16.gguf') if a.active_use else original
        if a.installed_target and not a.fail_use:
            expected = original.replace('engine = "parakeet"', 'engine = "gguf"')
            if a.nemotron_env:
                expected = expected.replace('Qwen3-ASR-0.6B-Q8_0.gguf', target_model).replace('architecture = "qwen3_asr"', 'architecture = "parakeet"')
        assert config.read_bytes() == expected.encode(), 'unexpected configuration mutation'
        calls = [json.loads(x) for x in commands.read_text().splitlines()]
        writes = [x for x in calls if x[:2] in [['config', 'set'], ['config', 'select-model']] or x[:1] == ['setup']]
        expected_writes = [['config', 'set', 'parakeet.model', 'parakeet-tdt-0.6b-v3-F16.gguf']] if a.active_use else []
        if a.installed_target:
            expected_writes = [['config','select-model','gguf',target_model]]
        passed = result.returncode == 0 and 'MINIMAL_BROWSE_PASS' in result.stdout + result.stderr and writes == expected_writes
        (a.out / 'results.json').write_text(json.dumps({'passed': passed, 'installed_target': a.installed_target, 'active_use': a.active_use, 'config_matches_expected_bytes': True, 'writes_or_downloads': writes, 'actual_rc2': True}, indent=2))
        assert passed, result.stdout + result.stderr
    finally:
        compositor.terminate()
        try:
            compositor.wait(timeout=5)
        except subprocess.TimeoutExpired:
            compositor.kill()
            compositor.wait()
print('MINIMAL_BROWSE_PASS')
