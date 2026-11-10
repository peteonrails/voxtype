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
    config.write_text('# graphical legacy fixture\nengine = "whisper"\n[whisper]\nmodel = "base.en" # keep\n[gguf]\nmodel = "Qwen3-ASR-0.6B-Q8_0.gguf"\narchitecture = "qwen3_asr"\nlanguage = "de"\n')
    store = home / ".local/share/voxtype/models"
    store.mkdir(parents=True)
    variants = ["Qwen3-ASR-0.6B-Q8_0.gguf", "Qwen3-ASR-1.7B-Q8_0.gguf", "moonshine-streaming-small-Q8_0.gguf", "gigaam-v3-e2e-rnnt-Q8_0.gguf", "nemotron-3.5-asr-streaming-0.6b-Q8_0.gguf"]
    for name in variants:
        (store / name).symlink_to((a.models / name).resolve())
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
        path.write_text(path.read_text().replace("id: root", "id: root\n  readonly property alias formUnderTest: form\n  readonly property alias cliUnderTest: cli", 1))
        path = root / "plugin/components/SchemaForm.qml"
        path.write_text(path.read_text().replace("id: root", "id: root\n  readonly property alias enginePickerUnderTest: testEnginePicker\n  readonly property alias modelCardUnderTest: testModelCard", 1).replace("EnginePicker {", "EnginePicker {\n      id: testEnginePicker", 1).replace("EngineModelCard {", "EngineModelCard {\n      id: testModelCard", 1))
        path = root / "plugin/components/EnginePicker.qml"
        path.write_text(path.read_text().replace("id: root", "id: root\n  readonly property alias dropdownUnderTest: engineDropdown", 1))
        path = root / "plugin/components/EngineModelCard.qml"
        path.write_text(path.read_text().replace("id: root", '''id: root
  function clickModelUnderTest(name) {
    for (var i = 0; i < testRepeater.count; i++) {
      var row = testRepeater.itemAt(i)
      if (row.modelName === name) { row.useButtonUnderTest.clicked(); return true }
    }
    return false
  }''', 1).replace("Repeater {", "Repeater {\n          id: testRepeater", 1).replace("id: entry", "id: entry\n            readonly property alias useButtonUnderTest: testUseButton", 1).replace('text: "Use"', 'id: testUseButton\n                  text: "Use"', 1))
        wrapper = root / "voxtype-test"
        wrapper.write_text("#!/usr/bin/python3\nimport os,sys\nos.execv(" + repr(str(binary)) + ",[" + repr(str(binary)) + ",\"--config\"," + repr(str(config)) + "]+sys.argv[1:])\n")
        wrapper.chmod(0o755)
        (root / "plugin/dev.json").write_text(json.dumps({"voxtypeBin": str(wrapper)}))
        qml = '''import QtQuick
import Quickshell
import "plugin" as Plugin
Item {
 id: test
 Plugin.Panel { id: panel; manifest: ({__sourceDir: PLUGIN_DIR}) }
 property int phase: 0
 property var variants: VARIANTS
 property var verified: []
 property bool finished: false
 function finish(message) { if (finished) return; finished = true; console.log(message === "" ? "HANDY_QML_PASS" : "HANDY_QML_FAIL: " + message); Qt.callLater(Qt.quit) }
 Connections {
  target: panel.cliUnderTest
  function onSchemaFailed(message) { test.finish(message) }
  function onWriteFailed(key, message) { test.finish(key + ": " + message) }
  function onSchemaLoaded(schema) {
   if (test.phase === 0) { test.phase = 1; panel.formUnderTest.enginePickerUnderTest.dropdownUnderTest.changed("gguf") }
   else {
    if (test.phase === 1) {
     var hint = ""
     for (var i = 0; i < schema.keys.length; i++)
      if (schema.keys[i].key === "gguf.language") hint = String(schema.keys[i].value)
     if (hint !== "de") { test.finish("saved language reset: " + hint); return }
     console.log("HANDY_LANGUAGE_PRESERVED: de")
    }
    if (panel.engine !== "gguf" || panel.modelValue !== test.variants[test.phase - 1]) { test.finish("engine/model pair " + panel.engine + "/" + panel.modelValue); return }
    test.verified.push(panel.modelValue)
    console.log("HANDY_VARIANT_PASS: " + panel.modelValue)
    if (test.phase === test.variants.length) { test.finish(""); return }
    var next = test.variants[test.phase]
    test.phase++
    if (!panel.formUnderTest.modelCardUnderTest.clickModelUnderTest(next)) test.finish("model absent " + next)
   }
  }
 }
 Component.onCompleted: Qt.callLater(function() { panel.open('{"section":"Engine"}') })
 Timer { interval: 20000; running: !test.finished; onTriggered: test.finish("timeout phase " + test.phase) }
}
'''.replace("PLUGIN_DIR", json.dumps(str(root / "plugin"))).replace("VARIANTS", json.dumps(variants))
        (root / "shell.qml").write_text(qml)
        result = subprocess.run(["qs", "-p", str(root / "shell.qml")], env=env, capture_output=True, text=True, timeout=25)
        (a.out / "stdout").write_text(result.stdout)
        (a.out / "stderr").write_text(result.stderr)
        (a.out / "exit").write_text(f"{result.returncode}\n")
        (a.out / "config.toml").write_text(config.read_text())
        assert result.returncode == 0 and "HANDY_QML_PASS" in result.stdout + result.stderr, (result.stdout, result.stderr)
        saved = tomllib.loads(config.read_text())
        assert saved["engine"] == "gguf"
        assert saved["gguf"]["model"] == variants[-1]
        assert saved["gguf"]["architecture"] == "parakeet"
        assert all("HANDY_VARIANT_PASS: " + name in result.stdout + result.stderr for name in variants)
        assert "HANDY_LANGUAGE_PRESERVED: de" in result.stdout + result.stderr
        (a.out / "results.json").write_text(json.dumps({"passed": True, "models": variants, "scope": "actual Panel/SchemaForm/EnginePicker and model Use handlers via VoxtypeCli, private headless compositor"}, indent=2) + "\n")
    finally:
        if compositor.poll() is None:
            compositor.terminate()
        try:
            compositor.wait(timeout=5)
        except subprocess.TimeoutExpired:
            compositor.kill()
            compositor.wait()
        log.close()

print("HANDY_QML_PASS")
