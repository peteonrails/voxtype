#!/usr/bin/env python3
"""Actual installed registry/scanner and verbatim panel Loader in isolation.

Run inside integrated-evidence/runtime.sh ui. No production shell is started.
Shell service/bar/auth APIs unused by this plugin are inert; its actual registry,
manifest sanitizer, panel-list reconciliation, Loader and summon implementation
are copied verbatim with source hashes. The panel window alone stays invisible.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import time

p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--bundle', type=Path, required=True)
p.add_argument('--evidence', type=Path, required=True)
p.add_argument('--shell', type=Path, default=Path('/usr/share/omarchy/shell'))
p.add_argument('--direct', action='store_true', help='negative control: bypass the canonical wrapper')
p.add_argument('--cache-override', help='verify a pre-existing cache override is preserved')
p.add_argument('--dev-mode', choices=['valid', 'missing', 'malformed', 'wrong-type'], default='valid')
a = p.parse_args()
bundle, out = a.bundle.resolve(), a.evidence.resolve()
out.mkdir(parents=True, exist_ok=True)
root = out / 'isolated-omarchy'
shell = root / 'shell'
shell.mkdir(parents=True, exist_ok=True)
source = a.shell / 'shell.qml'
original = source.read_text()
# Preserve and execute the installed source, not a reimplementation of stamping.
def between(start, end):
    return original[original.index(start):original.index(end)]
public = between('  function publicPluginManifest(', '  function publicBarConfig(')
loader = between('  property var openPanelIds:', '  // ---------------------------------------------------------- plugin loader')
reload_code = between('  function reloadPlugins()', '  function setPluginWidgetComponent(')
for child in a.shell.iterdir():
    if child.is_dir() and child.name != 'plugin':
        dest = shell / child.name
        if not dest.exists():
            dest.symlink_to(child.resolve(), target_is_directory=True)
plugins = out / 'plugins'
plugins.mkdir(exist_ok=True)
plugin = plugins / 'io.voxtype.settings'
shutil.copytree(bundle / 'omarchy-plugin', plugin, dirs_exist_ok=True)
panel = plugin / 'Panel.qml'
text = panel.read_text().replace('id: root', 'id: root\n  readonly property alias cliUnderTest: cli\n  property string fixtureRevision: "before"', 1)
assert 'visible: root.opened' in text
text = text.replace('visible: root.opened', 'visible: false // isolated: never map or focus a user window', 1)
panel.write_text(text)
config = out / 'config.toml'
config.write_text('# Disposable real loader config\nengine="whisper"\n[tray]\nenabled=false\n')
observations = out / 'backend-observations.jsonl'
# Interpose only an observer at the copied wrapper's exec boundary. It invokes
# the real candidate and reads /proc while it runs; no fabricated CLI responses.
observer = out / 'observe-backend'
observer.write_text('''#!/usr/bin/python3
import os,sys,json,subprocess,time,signal
from pathlib import Path
binary=Path(BINARY).resolve()
args=[str(binary),'--config',CONFIG]+sys.argv[1:]
p=subprocess.Popen(args)
# Popen returns after exec. Stop this disposable child before its short CLI
# exits so /proc exe, environment and cmdline are a coherent observation.
os.kill(p.pid,signal.SIGSTOP)
os.waitpid(p.pid,os.WUNTRACED)
record={'argv':args,'pid':p.pid,'environment':None,'exe':None}
for i in range(1000):
 try:
  exe=os.readlink('/proc/%d/exe'%p.pid)
  if exe==str(binary):
   record['exe']=exe
   env=Path('/proc/%d/environ'%p.pid).read_bytes().split(b'\\0')
   record['environment']={x.split(b'=',1)[0].decode():x.split(b'=',1)[1].decode() for x in env if b'=' in x and x.split(b'=',1)[0] in (b'ORT_MIGRAPHX_MODEL_CACHE_PATH',b'HOME',b'XDG_CACHE_HOME')}
   record['cmdline']=Path('/proc/%d/cmdline'%p.pid).read_bytes().decode().split('\\0')
   break
 except (OSError,ProcessLookupError): pass
 if p.poll() is not None: break
 time.sleep(.0005)
os.kill(p.pid,signal.SIGCONT)
rc=p.wait()
record['exit']=rc
with open(LOG,'a') as f:f.write(json.dumps(record)+'\\n')
sys.exit(rc)
'''.replace('BINARY', repr(str(bundle / 'voxtype'))).replace('CONFIG', repr(str(config))).replace('LOG', repr(str(observations))))
observer.chmod(0o755)
wrapper = out / 'canonical-wrapper'
# Preserve exactly the cache setup from the actual daily wrapper. Substitute
# only its immutable target in this disposable copy; production is untouched.
daily = Path('/usr/local/bin/voxtype').read_text()
old_exec = 'exec /usr/local/lib/voxtype/migraphx/voxtype-onnx-migraphx "$@"'
assert old_exec in daily
wrapper.write_text(daily.replace(old_exec, 'exec ' + str(observer) + ' "$@"'))
wrapper.chmod(0o755)
binding = observer if a.direct else wrapper
dev = plugin / 'dev.json'
if a.dev_mode == 'valid':
    dev.write_text(json.dumps({'voxtypeBin': str(binding)}))
elif a.dev_mode == 'missing':
    dev.unlink(missing_ok=True)
else:
    dev.write_text('{' if a.dev_mode == 'malformed' else '{"voxtypeBin":42}')
expected_bin = str(binding) if a.dev_mode == 'valid' else 'voxtype'
fallback_bin = out / 'bin'
fallback_bin.mkdir(exist_ok=True)
(fallback_bin / 'voxtype').symlink_to(binding)
qml = '''import QtQuick
import Quickshell
import Quickshell.Io
import QtQml.Models
import "services" as Services
Item {
 id: shell
 property string omarchyPath: ROOT
 property bool pluginReloading: false
 property bool pluginReloadPending: false
 Timer { id: localPluginReloadTimer; interval: 150; onTriggered: shell.reloadPlugins() }
 function unloadPluginServices() {}
 function unloadPluginWidgets() {}
 function _syncServices() {}
 function syncPluginWidgets() {}
 property var pluginRegistry: registry
 property var bar: null
 property bool queuedFirstOpen: false
 Connections {
  target: registry
  function onScanFinished() {
   if (!shell.queuedFirstOpen) {
    shell.queuedFirstOpen = true
    shell.summon("io.voxtype.settings", "{}")
   }
  }
 }
 Services.PluginRegistry {
  id: registry
  pluginsDir: PLUGINS
  firstPartyDir: ""
  shellConfigProvider: function() { return {plugins:[{id:"io.voxtype.settings"}]} }
 }
 function pluginShellFor(m) { return shell }
 function pluginBarWidgetRegistryFor(m) { return null }
 function pluginRegistryFor(m) { return null }
 function serviceFor(id) { return null }
 PUBLIC
 LOADER
 RELOAD_CODE
 IpcHandler {
  target: "loader-test"
  function state(): string {
   var l = shell.panelLoaders["io.voxtype.settings"]
   var p = l ? l.item : null
   return JSON.stringify({scanning:registry.scanning, mounted:!!p, opened:p ? p.opened : false,
     revision:p ? p.fixtureRevision : "", manifest:p ? p.manifest : null,
     bin:p ? p.cliUnderTest.bin : "", sections:p ? p.sections : [], config:p ? p.configPath : "",
     rows:panelEntryModel.count})
  }
  function markInstance(): string { shell.panelLoaders["io.voxtype.settings"].item.fixtureRevision = "instance-only"; return "marked" }
  function rescan(): string { registry.rescan(); return "requested" }
  function reload(): string { shell.reloadPlugins(); return "requested" }
  function quit(): string { Qt.callLater(Qt.quit); return "bye" }
 }
}
'''.replace('ROOT', json.dumps(str(root))).replace('PLUGINS', json.dumps(str(plugins))).replace('PUBLIC', public).replace('LOADER', loader).replace('RELOAD_CODE', reload_code)
(shell / 'shell.qml').write_text(qml)
(out / 'loader-source.json').write_text(json.dumps({str(f):hashlib.sha256(f.read_bytes()).hexdigest() for f in (source, a.shell / 'services/PluginRegistry.qml', Path('/usr/local/bin/voxtype'))}, indent=2))
env = dict(os.environ, OMARCHY_PATH=str(root))
if a.dev_mode != 'valid':
    env['PATH'] = str(fallback_bin) + ':' + env['PATH']
# Deliberately omit the cache variable: the copied daily wrapper must supply it.
env.pop('ORT_MIGRAPHX_MODEL_CACHE_PATH', None)
if a.cache_override:
    env['ORT_MIGRAPHX_MODEL_CACHE_PATH'] = a.cache_override
results = []
proc = None
log = None
def ipc(target, method):
    return subprocess.run(['omarchy-shell', target, method], env=env, text=True, capture_output=True, timeout=5)
def state():
    r = ipc('loader-test', 'state')
    return json.loads(r.stdout) if r.returncode == 0 else {}
def wait_for(predicate):
    assert proc is not None
    deadline = time.monotonic() + 25
    last = {}
    while time.monotonic() < deadline:
        assert proc.poll() is None, (out / 'quickshell.log').read_text()
        try:
            last = state()
            if predicate(last):
                return last
        except (json.JSONDecodeError, subprocess.TimeoutExpired):
            pass
        time.sleep(.1)
    raise AssertionError(last)
def start():
    global proc, log
    log = (out / 'quickshell.log').open('a')
    proc = subprocess.Popen(['qs', '-p', str(shell)], env=env, stdout=log, stderr=subprocess.STDOUT)
def stop():
    if proc is not None:
        assert log is not None
        ipc('loader-test', 'quit')
        try:
            proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            proc.kill(); proc.wait()
        log.close()
        assert proc.returncode == 0
def schema_records():
    if not observations.exists():
        return []
    return [r for r in (json.loads(line) for line in observations.read_text().splitlines())
            if r['argv'][-3:] == ['config', 'schema', '--json']]
def wait_schema_count(count):
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        if len(schema_records()) >= count:
            return
        time.sleep(.05)
    raise AssertionError(('schema subprocess did not complete', count, schema_records()))
try:
    start()
    initial = wait_for(lambda s: s.get('mounted') and s['bin'] == expected_bin)
    assert initial['manifest']['__sourceDir'] == str(plugin), initial
    assert '__isFirstParty' not in initial['manifest'], initial
    assert initial['rows'] == 1 and initial['opened'], initial
    wait_for(lambda s: s.get('config') == str(config))
    wait_schema_count(1)
    results.append({'actual_scanner_loader_manifest_and_queued_open': initial})
    for n in range(2):
        r = subprocess.run([str(bundle / 'voxtype-configure-launcher')], env=env, text=True, capture_output=True, timeout=10)
        assert r.returncode == 0, (r.stdout, r.stderr)
        opened = wait_for(lambda s: s.get('opened') and 'Text Cleanup' in s['sections'] and s['config'] == str(config))
        assert opened['rows'] == 1
        wait_schema_count(1)
        results.append({'idempotent_open': n, 'state': opened})
    # Rescan retains an unchanged keepLoaded instance, including its state.
    # Mark only memory here so the real file watcher cannot turn the rescan
    # into a full reload before the assertion.
    assert ipc('loader-test', 'markInstance').stdout.strip() == 'marked'
    r = ipc('loader-test', 'rescan')
    assert r.stdout.strip() == 'requested'
    time.sleep(.3)
    stale = wait_for(lambda s: not s.get('scanning') and s.get('mounted'))
    assert stale['revision'] == 'instance-only', stale
    results.append({'rescan_retains_cached_instance': stale})
    assert ipc('loader-test', 'reload').stdout.strip() == 'requested'
    reloaded = wait_for(lambda s: s.get('mounted') and s['revision'] == 'before')
    results.append({'actual_full_plugin_reload_recreates_instance': reloaded})
    # Now change disk bytes and let the installed registry watcher and actual
    # host reload path react. Qt may retain the compiled component cache.
    panel.write_text(panel.read_text().replace('fixtureRevision: "before"', 'fixtureRevision: "after"'))
    time.sleep(1)
    after_write = wait_for(lambda s: s.get('mounted') and not s.get('scanning'))
    assert after_write['revision'] in ('before', 'after'), after_write
    results.append({'actual_watch_reload_after_qml_write': after_write})
    stop(); proc = None
    start()
    fresh = wait_for(lambda s: s.get('mounted') and s.get('revision') == 'after')
    assert fresh['bin'] == expected_bin
    r = ipc('io.voxtype.settings', 'open')
    assert r.stdout.strip() == 'ok', r
    wait_for(lambda s: s.get('opened') and s.get('config') == str(config))
    wait_schema_count(2)
    results.append({'isolated_restart_loads_current_bytes': fresh})
finally:
    stop()
qml_log = (out / 'quickshell.log').read_text()
assert 'Local plugin changed, reloading:' in qml_log, qml_log
assert not any(error in qml_log for error in ('TypeError:', 'ReferenceError:', 'Cannot assign to non-existent property')), qml_log
records = [json.loads(line) for line in observations.read_text().splitlines()]
schemas = [r for r in records if r['argv'][-3:] == ['config', 'schema', '--json']]
assert len(schemas) == 2, records
for record in schemas:
    assert record['exit'] == 0 and record['exe'] == str((bundle / 'voxtype').resolve()), record
    assert record['cmdline'][1:3] == ['--config', str(config)], record
    cache = record['environment'].get('ORT_MIGRAPHX_MODEL_CACHE_PATH')
    assert cache == (a.cache_override or str(Path(env['HOME']) / '.cache/voxtype/migraphx')), record
    assert Path(cache).is_dir(), cache
results.append({'observed_candidate_schema_processes': len(schemas), 'records': schemas})
(out / 'result.json').write_text(json.dumps(results, indent=2) + '\n')
print('REAL_QS_LOADER_PASS: actual scanner/registry, verbatim host Loader/summon/public manifest, dev.json, observed backend/config/cache, idempotent opens, stale rescan and isolated restart')
