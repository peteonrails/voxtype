#!/usr/bin/env python3
"""Exercise the real Omarchy QML engine and model pickers for shared GGUF."""
import argparse, json, os, pathlib, re, shutil, subprocess, tempfile, time, tomllib

p = argparse.ArgumentParser()
p.add_argument("--bin", required=True, type=pathlib.Path)
p.add_argument("--models", required=True, type=pathlib.Path)
p.add_argument("--plugin", required=True, type=pathlib.Path)
p.add_argument("--tools", required=True, type=pathlib.Path)
p.add_argument("--out", required=True, type=pathlib.Path)
a = p.parse_args()
if (os.environ.get("RC2_DESKTOP_ISOLATED") != "1"
        or pathlib.Path("/proc/1/comm").read_text().strip() != "bwrap"
        or pathlib.Path("/run/user").exists()
        or pathlib.Path("/dev/input").exists()
        or pathlib.Path("/dev/uinput").exists()):
    p.error("Requires the verified desktop-isolated runtime wrapper; host execution is forbidden")
a.out.mkdir(parents=True, exist_ok=True)
binary = a.bin.resolve()

with tempfile.TemporaryDirectory(dir=a.out) as temporary:
    root = pathlib.Path(temporary)
    home = root / "home"
    runtime = root / "runtime"
    home.mkdir()
    runtime.mkdir(parents=True, exist_ok=True)
    runtime.chmod(0o700)
    config = root / "config.toml"
    original = '# keep active selection\nengine = "parakeet"\n[parakeet]\nmodel = "parakeet-ultra-F16.gguf" # keep\n[gguf]\nmodel = "Qwen3-ASR-0.6B-Q8_0.gguf"\narchitecture = "qwen3_asr"\nlanguage = "de"\n'
    config.write_text(original)
    store = home / ".local/share/voxtype/models"
    store.mkdir(parents=True)
    (store / "parakeet-ultra-F16.gguf").symlink_to(a.models / "parakeet-ultra-F16.gguf")
    variants = ["Qwen3-ASR-0.6B-Q8_0.gguf", "Qwen3-ASR-1.7B-Q8_0.gguf", "moonshine-streaming-small-Q8_0.gguf", "gigaam-v3-e2e-rnnt-Q8_0.gguf", "nemotron-3.5-asr-streaming-0.6b-Q8_0.gguf"]
    worker = binary.with_name("voxtype-gguf-worker-vulkan")
    assert worker.is_file()
    env = dict(os.environ, HOME=str(home), XDG_RUNTIME_DIR=str(runtime), XDG_CONFIG_HOME=str(home / ".config"), XDG_CACHE_HOME=str(home / ".cache"), XDG_DATA_HOME=str(home / ".local/share"), WAYLAND_DISPLAY="", DISPLAY="", HYPRLAND_INSTANCE_SIGNATURE="", WLR_BACKENDS="headless", WLR_RENDERER="pixman", LD_LIBRARY_PATH=str(a.tools / "lib"), QT_QPA_PLATFORM="wayland", QT_QUICK_BACKEND="software", VOXTYPE_GGUF_WORKER=str(worker), DBUS_SESSION_BUS_ADDRESS=f"unix:path={runtime}/absent-bus")
    env.pop("WAYLAND_DISPLAY", None)
    log = (a.out / "compositor.log").open("w")
    compositor = subprocess.Popen([str(a.tools / "bin/labwc"), "-C", str(root / "labwc-empty")], env=env, stdout=log, stderr=log, start_new_session=True)
    try:
        sockets = []
        for _ in range(100):
            sockets = [path for path in runtime.glob("wayland-*") if not path.name.endswith(".lock")]
            if sockets or compositor.poll() is not None:
                break
            time.sleep(0.1)
        assert sockets and compositor.poll() is None, "private compositor failed"
        env["WAYLAND_DISPLAY"] = sockets[0].name
        for directory in pathlib.Path("/usr/share/omarchy/shell").iterdir():
            if directory.is_dir():
                (root / directory.name).symlink_to(directory)
        shutil.copytree(a.plugin, root / "plugin", ignore=shutil.ignore_patterns("__pycache__"))
        path = root / "plugin/Panel.qml"
        path.write_text(path.read_text().replace("id: root", "id: root\n  readonly property alias formUnderTest: form\n  readonly property alias cliUnderTest: cli\n  readonly property alias renderUnderTest: settingsCard.card", 1))
        path = root / "plugin/components/SchemaForm.qml"
        path.write_text(path.read_text().replace("id: root", "id: root\n  readonly property alias enginePickerUnderTest: testEnginePicker\n  readonly property alias modelCardUnderTest: testModelCard", 1).replace("EnginePicker {", "EnginePicker {\n      id: testEnginePicker", 1).replace("EngineModelCard {", "EngineModelCard {\n      id: testModelCard", 1))
        path = root / "plugin/components/EnginePicker.qml"
        path.write_text(path.read_text().replace("id: root", "id: root\n  readonly property alias dropdownUnderTest: engineDropdown", 1))
        path = root / "plugin/components/EngineModelCard.qml"
        path.write_text(path.read_text().replace("id: root", '''id: root
  property var pointerUnderTest: null
  function downloadUnderTest(name) {
    for (var i = 0; i < testRepeater.count; i++) {
      var row = testRepeater.itemAt(i)
      if (row.modelName === name) { if (!row.downloadButtonUnderTest.visible || !row.downloadButtonUnderTest.enabled) return false; root.pointerUnderTest.mouseClick(row.downloadButtonUnderTest, row.downloadButtonUnderTest.width / 2, row.downloadButtonUnderTest.height / 2); return true }
    }
    return false
  }
  function clickModelUnderTest(name) {
    for (var i = 0; i < testRepeater.count; i++) {
      var row = testRepeater.itemAt(i)
      if (row.modelName === name) { root.pointerUnderTest.mouseClick(row.useButtonUnderTest, row.useButtonUnderTest.width / 2, row.useButtonUnderTest.height / 2); return true }
    }
    return false
  }''', 1).replace("Repeater {", "Repeater {\n          id: testRepeater", 1).replace("id: entry", "id: entry\n            readonly property alias useButtonUnderTest: testUseButton\n            readonly property alias downloadButtonUnderTest: testDownloadButton", 1).replace('text: "Use"', 'id: testUseButton\n                  text: "Use"', 1).replace('text: "Download"', 'id: testDownloadButton\n                  text: "Download"', 1))
        wrapper = root / "voxtype-test"
        wrapper.write_text(r"""#!/usr/bin/python3
import json,os,pathlib,sys,time
root=pathlib.Path(__file__).parent
args=sys.argv[1:]
with (root/'cli.jsonl').open('a') as f:f.write(json.dumps(args)+'\n')
if args[:2] == ['setup','--download']:
    assert '--no-post-install' in args and '--activate' not in args
    n=int((root/'attempts').read_text())+1 if (root/'attempts').exists() else 1
    (root/'attempts').write_text(str(n))
    print(json.dumps({'event':'progress','pct':40,'file':'fixture'}),flush=True)
    if n == 1:
        time.sleep(.3)
        print(json.dumps({'event':'error','message':'Fixture transport failed. Retry Download.'}),flush=True)
        sys.exit(1)
    if n == 2: time.sleep(60);sys.exit(1)
    target=pathlib.Path(os.environ['XDG_DATA_HOME'])/'voxtype/models/Qwen3-ASR-0.6B-Q8_0.gguf'
    target.symlink_to('/models/Qwen3-ASR-0.6B-Q8_0.gguf')
    print(json.dumps({'event':'done'}),flush=True)
    sys.exit(0)
os.execv(BINARY,[BINARY,'--config',str(root/'config.toml')]+args)
""".replace("BINARY", repr(str(binary))))
        wrapper.chmod(0o755)
        (root / "plugin/dev.json").write_text(json.dumps({"voxtypeBin": str(wrapper)}))
        qml = r"""import QtQuick
import QtTest
import Quickshell
import "plugin" as Plugin
Item {
 id: test
 Plugin.Panel { id: panel; manifest: ({__sourceDir: PLUGIN_DIR}) }
 TestCase { id: pointer; parent: panel.renderUnderTest; when: false }
 property int phase: 0
 property int browseIndex: 0
 property string wanted: ""
 property bool finished: false
 property string model: "Qwen3-ASR-0.6B-Q8_0.gguf"
 function finish(message) { if (finished) return; finished = true; console.log(message === "" ? "WORKFLOW_QML_PASS" : "WORKFLOW_QML_FAIL: " + message); Qt.callLater(Qt.quit) }
 Connections { target: panel.cliUnderTest
   function onWriteFailed(key, message) { test.finish("Use failed: " + message) }
   function onDownloadFinished(model, ok, message) {
     if (test.phase === 3) {
       if (ok || message.indexOf("Fixture transport failed") === -1) { test.finish("wrong failure"); return }
       test.phase = 4
     } else if (test.phase === 5) {
       if (ok || message !== "") { test.finish("cancel reported failure/success"); return }
       test.phase = 6
     } else if (test.phase === 7) {
       if (!ok) { test.finish("retry failed: " + message); return }
       test.phase = 8
     }
   }
 }
 Timer { interval: 150; running: !test.finished; repeat: true; onTriggered: {
   var picker = panel.formUnderTest.enginePickerUnderTest
   var card = panel.formUnderTest.modelCardUnderTest
   card.pointerUnderTest = pointer
   if (test.phase > 0 && test.phase < 9 && panel.engine !== "parakeet") { test.finish("active engine changed before Use"); return }
   if (test.phase === 0) {
     if (!panel.engineChoices.length || !Object.keys(panel.engineAvailability).length || !card.modelRows.length) return
     test.phase = 1
   }
   if (test.phase === 1) {
     if (test.wanted !== "") {
       if (card.engine !== test.wanted) { test.finish("catalog snapped back " + test.wanted); return }
       var expected = panel.engines[test.wanted]
       if (expected && card.modelRows.length !== expected.models.length) { test.finish("catalog incomplete " + test.wanted); return }
       console.log("BROWSE_ENGINE_PASS: " + test.wanted)
     }
     while (test.browseIndex < panel.engineChoices.length) {
       var next = String(panel.engineChoices[test.browseIndex++])
       if (panel.engineAvailability[next] === false) continue
       test.wanted = next; picker.dropdownUnderTest.changed(next); return
     }
     picker.dropdownUnderTest.changed("gguf"); test.phase = 2; return
   }
   if (test.phase === 2) {
     if (card.engine !== "gguf" || !card.downloadUnderTest(test.model)) { test.finish("Download not reachable"); return }
     test.phase = 3; return
   }
   if (test.phase === 4) {
     if (card.engine !== "gguf" || !panel.downloadErrors["gguf:" + test.model]) { test.finish("error/catalog lost"); return }
     if (!card.downloadUnderTest(test.model)) { test.finish("retry inaccessible"); return }
     test.phase = 5; return
   }
   if (test.phase === 5 && panel.cliUnderTest.downloadPct === 40) {
     picker.dropdownUnderTest.changed("whisper")
     if (card.engine !== "gguf" || picker.dropdownUnderTest.enabled) { test.finish("download lost engine identity"); return }
     card.cancelDownloadRequested(); return
   }
   if (test.phase === 6) {
     if (card.engine !== "gguf") { test.finish("cancel lost catalog"); return }
     if (!card.downloadUnderTest(test.model)) { test.finish("cancel retry inaccessible"); return }
     test.phase = 7; return
   }
   if (test.phase === 8) {
     var installed = false
     for (var i = 0; i < card.modelRows.length; i++) if (card.modelRows[i].name === test.model) installed = card.modelRows[i].installed
     if (!installed) return
     console.log("DOWNLOAD_ONLY_DID_NOT_ACTIVATE")
     test.phase = 9
     if (!card.clickModelUnderTest(test.model)) test.finish("Use not reachable")
     return
   }
   if (test.phase === 9 && panel.engine === "gguf") {
     if (panel.modelValue !== test.model) { test.finish("wrong model after Use"); return }
     test.phase = 10
     panel.renderUnderTest.grabToImage(function(result) { result.saveToFile(CAPTURE); test.finish("") })
   }
 } }
 Component.onCompleted: Qt.callLater(function() { panel.open('{"section":"Engine"}') })
 Timer { interval: 18000; running: !test.finished; onTriggered: test.finish("timeout phase " + test.phase) }
}
""".replace("PLUGIN_DIR", json.dumps(str(root / "plugin"))).replace("CAPTURE", json.dumps(str(a.out / "used.png")))
        (root / "shell.qml").write_text(qml)
        result = subprocess.run(["qs", "-p", str(root / "shell.qml")], env=env, capture_output=True, text=True, timeout=25)
        (a.out / "stdout").write_text(result.stdout)
        (a.out / "stderr").write_text(result.stderr)
        (a.out / "exit").write_text(f"{result.returncode}\n")
        (a.out / "config.toml").write_text(config.read_text())
        assert result.returncode == 0 and "WORKFLOW_QML_PASS" in result.stdout + result.stderr, (result.stdout, result.stderr)
        saved = tomllib.loads(config.read_text())
        assert saved['engine'] == 'gguf' and saved['gguf']['model'] == variants[0]
        assert saved['gguf']['language'] == 'de' and saved['gguf']['architecture'] == 'qwen3_asr'
        assert saved['parakeet'] == tomllib.loads(original)['parakeet'] and '# keep' in config.read_text()
        shutil.copy2(root / 'cli.jsonl', a.out / 'cli.jsonl')
        assert (root / 'attempts').read_text() == '3'
        (a.out / "results.json").write_text(json.dumps({"passed": True, "models": variants, "scope": "actual QML on private labwc: compiled catalogs, fixture download failure/progress/cancel/retry, download-only nonactivation, real candidate CLI atomic Use"}, indent=2))
    finally:
        if compositor.poll() is None:
            compositor.terminate()
        try:
            compositor.wait(timeout=5)
        except subprocess.TimeoutExpired:
            compositor.kill()
            compositor.wait()
        log.close()

print("WORKFLOW_QML_PASS")
