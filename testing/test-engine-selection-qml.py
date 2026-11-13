#!/usr/bin/env python3
"""Exercise real EnginePicker -> Panel -> VoxtypeCli -> candidate persistence.
Runs full QML on a private headless labwc, never the production compositor/bus.
"""
import argparse, json, os, pathlib, re, shutil, subprocess, tempfile, time, tomllib
p=argparse.ArgumentParser(description=__doc__);p.add_argument('--bin',required=True,type=pathlib.Path);p.add_argument('--models',required=True,type=pathlib.Path);p.add_argument('--plugin',type=pathlib.Path);p.add_argument('--tools',type=pathlib.Path);p.add_argument('--evidence',required=True,type=pathlib.Path);a=p.parse_args()
r=pathlib.Path(__file__).resolve().parents[1]; evidence=a.evidence.resolve();evidence.mkdir(parents=True,exist_ok=True);binary=a.bin.resolve();plugin=(a.plugin or r/'omarchy-plugin').resolve();tools=(a.tools or r/'selection-evidence/compositor-tools/usr').resolve()
assert os.environ.get('XDG_RUNTIME_DIR')=='/run/user/1000' and os.environ.get('DBUS_SESSION_BUS_ADDRESS','').endswith('absent-bus')
models={'parakeet':'parakeet-ultra-Q8_0.gguf','cohere':'cohere-transcribe-03-2026-Q4_K_M.gguf'};legacy={'parakeet':'parakeet-tdt-0.6b-v2','cohere':'cohere-transcribe-int8'}
children=[]; results=[]
with tempfile.TemporaryDirectory(dir=evidence) as temporary:
 root=pathlib.Path(temporary);home=root/'home';home.mkdir();env=dict(os.environ,HOME=str(home),XDG_CONFIG_HOME=str(home/'.config'),XDG_CACHE_HOME=str(home/'.cache'),XDG_DATA_HOME=str(home/'.local/share'),WAYLAND_DISPLAY='',DISPLAY='',HYPRLAND_INSTANCE_SIGNATURE='',WLR_BACKENDS='headless',WLR_RENDERER='gles2',WLR_RENDER_DRM_DEVICE='/dev/dri/renderD128',LD_LIBRARY_PATH=str(tools/'lib'),QT_QPA_PLATFORM='wayland',QT_QUICK_BACKEND='software')
 log=(evidence/'compositor.log').open('w');comp=subprocess.Popen([str(tools/'bin/labwc'),'-C',str(root/'labwc-empty')],env=env,stdout=log,stderr=log,start_new_session=True);children.append(comp)
 try:
  sockets=[]
  for _ in range(80):
   sockets=[f for f in pathlib.Path('/run/user/1000').glob('wayland-*') if not f.name.endswith('.lock')]
   if sockets or comp.poll() is not None:break
   time.sleep(.1)
  assert sockets and comp.poll() is None,'private compositor failed';env['WAYLAND_DISPLAY']=str(sockets[0])
  for engine,model in models.items():
   test=root/engine;test.mkdir();config=test/'config.toml';original=f'# graphical selection fixture\nengine = "{engine}"\n[parakeet]\nmodel = "{legacy["parakeet"]}" # preserve comment\nstreaming = false\nmodel_type = "tdt"\n[cohere]\nmodel = "{legacy["cohere"]}"\nlanguage = "en"\ngguf_backend = "vulkan"\n[whisper]\nlanguage = "de"\n';config.write_text(original)
   store=home/'.local/share/voxtype/models';store.mkdir(parents=True,exist_ok=True)
   for name in list(models.values())+['parakeet-ultra-F16.gguf','parakeet-ultra-Q4_K_M.gguf']:
    if not (store/name).exists():(store/name).symlink_to(a.models/name)
   for directory in pathlib.Path('/usr/share/omarchy/shell').iterdir():
    if directory.is_dir():(test/directory.name).symlink_to(directory)
   shutil.copytree(plugin,test/'plugin',ignore=shutil.ignore_patterns('__pycache__'))
   path=test/'plugin/Panel.qml';path.write_text(path.read_text().replace('id: root','id: root\n  readonly property alias formUnderTest: form\n  readonly property alias cliUnderTest: cli',1))
   path=test/'plugin/components/SchemaForm.qml';path.write_text(path.read_text().replace('id: root','id: root\n  readonly property alias enginePickerUnderTest: testEnginePicker\n  readonly property alias modelCardUnderTest: testModelCard',1).replace('EnginePicker {','EnginePicker {\n      id: testEnginePicker',1).replace('EngineModelCard {','EngineModelCard {\n      id: testModelCard',1))
   path=test/'plugin/components/EngineModelCard.qml';path.write_text(path.read_text().replace('id: root','''id: root
  function clickModelUnderTest(name) {
    for (var i = 0; i < testRepeater.count; i++) {
      var row = testRepeater.itemAt(i)
      if (row.modelName === name) { row.useButtonUnderTest.clicked(); return true }
    }
    return false
  }''',1).replace('Repeater {','Repeater {\n          id: testRepeater',1).replace('id: entry','id: entry\n            readonly property alias useButtonUnderTest: testUseButton',1).replace('text: "Use"','id: testUseButton\n                  text: "Use"',1))
   path=test/'plugin/components/EnginePicker.qml';path.write_text(path.read_text().replace('id: root','id: root\n  readonly property alias dropdownUnderTest: engineDropdown',1))
   wrapper=test/'voxtype-test';wrapper.write_text('#!/usr/bin/python3\nimport os,sys\nos.execv('+repr(str(binary))+',['+repr(str(binary))+',"--config",'+repr(str(config))+']+sys.argv[1:])\n');wrapper.chmod(0o755)
   (test/'plugin/dev.json').write_text(json.dumps({'voxtypeBin':str(wrapper)}))
   qml='''import QtQuick
import Quickshell
import "plugin" as Plugin
Item {
 id: test
 Plugin.Panel { id: panel; manifest: ({__sourceDir: PLUGIN_DIR}) }
 property int phase: 0
 property bool finished: false
 property string failure: ""
 property var ultraModels: ["parakeet-ultra-F16.gguf", "parakeet-ultra-Q4_K_M.gguf", "parakeet-ultra-Q8_0.gguf"]
 property int ultraIndex: 0
 function finish(message) { if (finished) return; finished = true; failure = message; console.log(message === "" ? "ENGINE_SELECTION_QS_PASS" : "ENGINE_SELECTION_QS_FAIL: " + message); Qt.callLater(Qt.quit) }
 Connections {
  target: panel.cliUnderTest
  function onSchemaFailed(message) { test.finish(message) }
  function onSchemaLoaded(schema) {
   if (test.phase === 0) { test.phase = 1; panel.cliUnderTest.restartDaemon() }
   else if (test.phase === 2) {
    if (panel.engine !== ENGINE || panel.modelValue !== MODEL) { test.finish("persisted pair mismatch " + panel.engine + "/" + panel.modelValue); return }
    if (ENGINE === "parakeet") {
      var card = panel.formUnderTest.modelCardUnderTest
      var names = card.modelRows.map(function(row) { return row.name })
      for (var i = 0; i < ultraModels.length; i++) if (names.indexOf(ultraModels[i]) === -1) { test.finish("Ultra model absent from graphical choices"); return }
      test.phase = 4; card.clickModelUnderTest(ultraModels[0])
    } else { test.phase = 3; panel.cliUnderTest.setKey(ENGINE + ".model", LEGACY) }
   } else if (test.phase === 4) {
    if (panel.modelValue !== ultraModels[ultraIndex]) { test.finish("model Use button did not persist: " + panel.modelValue); return }
    console.log("ULTRA_USE_QS_PASS: " + panel.modelValue)
    ultraIndex++
    if (ultraIndex < ultraModels.length) panel.formUnderTest.modelCardUnderTest.clickModelUnderTest(ultraModels[ultraIndex])
    else { test.phase = 3; panel.cliUnderTest.setKey(ENGINE + ".model", LEGACY) }
   }
  }
  function onRestarted(ok, message) {
   if (test.phase !== 1) return
   if (ok || message.indexOf("ONNX") === -1 || message.indexOf("GGUF") === -1) { test.finish("invalid saved pair allowed restart: " + message); return }
   test.phase = 2
   panel.formUnderTest.enginePickerUnderTest.dropdownUnderTest.changed(ENGINE)
  }
  function onWriteFailed(key,message) {
   if (test.phase !== 3 || key !== ENGINE + ".model" || message.indexOf("ONNX") === -1 || message.indexOf("GGUF") === -1) { test.finish("unexpected write failure: " + key + ": " + message); return }
   test.finish("")
  }
 }
 Component.onCompleted: Qt.callLater(function() { panel.open('{"section":"Engine"}') })
 Timer { interval: 20000; running: !test.finished; onTriggered: test.finish("timeout at phase " + test.phase) }
}
'''
   for token,value in [('PLUGIN_DIR',str(test/'plugin')),('ENGINE',engine),('MODEL',model),('LEGACY',legacy[engine])]:qml=re.sub(r'\b'+token+r'\b',lambda match:json.dumps(value),qml)
   (test/'shell.qml').write_text(qml)
   x=subprocess.run(['qs','-p',str(test/'shell.qml')],env=env,capture_output=True,text=True,timeout=25)
   (evidence/(engine+'.stdout')).write_text(x.stdout);(evidence/(engine+'.stderr')).write_text(x.stderr);(evidence/(engine+'.exit')).write_text(str(x.returncode)+'\n');(evidence/(engine+'.toml')).write_text(config.read_text())
   assert x.returncode==0 and 'ENGINE_SELECTION_QS_PASS' in x.stdout+x.stderr,(engine,x.stdout,x.stderr)
   assert 'ENGINE_SELECTION_QS_FAIL' not in x.stdout+x.stderr
   saved=tomllib.loads(config.read_text());assert saved['engine']==engine and saved[engine]['model']==model
   other='cohere' if engine=='parakeet' else 'parakeet';assert saved[other]==tomllib.loads(original)[other];assert saved['whisper']['language']=='de' and '# preserve comment' in config.read_text()
   results.append({'engine':engine,'passed':True,'scope':'actual Dropdown changed handler, Panel/VoxtypeCli persistence and rejected invalid-pair restart; private headless compositor'})
 finally:
  for child in reversed(children):
   if child.poll() is None:child.terminate()
   try:child.wait(timeout=5)
   except subprocess.TimeoutExpired:child.kill();child.wait()
  log.close();(evidence/'results.json').write_text(json.dumps(results,indent=2)+'\n')
print(json.dumps(results,indent=2))
