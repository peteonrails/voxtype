"""Actual Quickshell CLI transport on a private headless compositor.

Run only inside the approved offline NAS contributor container. This is not
installed Omarchy or physical-desktop acceptance.
"""
import datetime
import json
import os
from pathlib import Path
import select
import shutil
import stat
import subprocess
import tempfile
import time

assert os.environ.get('HOME') == '/work/home', 'NAS fixture only'
assert not any(os.environ.get(k) for k in ['DISPLAY','WAYLAND_DISPLAY','SWAYSOCK','HYPRLAND_INSTANCE_SIGNATURE','DBUS_SESSION_BUS_ADDRESS'])
os.umask(0o077)
source = Path(__file__).resolve().parents[1]
exe = Path(os.environ.get('VOXTYPE_HISTORY_TEST_BIN','/work/target/debug/voxtype'))
case = os.environ.get('VOX_HISTORY_QS_CASE','transport')
assert case in ['transport','window']
with tempfile.TemporaryDirectory(dir='/work/scratch',prefix='history-quickshell-') as name:
    root = Path(name)
    runtime = root/'runtime'
    runtime.mkdir(mode=0o700)
    config = root/'config/voxtype'
    config.mkdir(parents=True)
    (config/'config.toml').write_text('[hotkey]\nenabled=false\n[history]\nenabled=true\n')
    (root/'dev.json').write_text(json.dumps({'voxtypeBin':str(exe)}))
    # Preserve the real transport bytes while excluding unrelated Omarchy-only
    # siblings from Quickshell's eager module scanner. No transport stubs.
    components = root/'components'
    components.mkdir()
    for filename in ['VoxtypeCli.qml','CliProcess.qml','HistoryPanel.qml','HistoryWindow.qml']:
        original = source/'omarchy-plugin/components'/filename
        if original.is_file():
            shutil.copyfile(original,components/filename)
    day = datetime.datetime.now().astimezone()
    entry_id = day.strftime('%Y%m%d-%H%M%S')+'-abc123'
    entry = dict(schema=1,id=entry_id,ts_start=day.isoformat(),ts_end=day.isoformat(),raw_text='synthetic transport',text='Synthetic transport.',engine='whisper',model='synthetic.gguf',backend='cpu',variant='test',audio_ms=1000,transcribe_ms=10,streaming=False,voxtype_version='fixture',delivered=True,outcome='delivered',trigger='record',ended_by='released',auto_submit=False,edits=[])
    folder = root/'data/voxtype/history'
    folder.mkdir(parents=True,mode=0o700)
    (folder/(day.strftime('%Y-%m-%d')+'.jsonl')).write_text(json.dumps(entry)+'\n')
    env = dict(os.environ,HOME=str(root),XDG_CONFIG_HOME=str(root/'config'),XDG_DATA_HOME=str(root/'data'),XDG_RUNTIME_DIR=str(runtime),WLR_BACKENDS='headless',WLR_RENDERER='pixman',WLR_LIBINPUT_NO_DEVICES='1',WLR_HEADLESS_OUTPUTS='1',QT_QPA_PLATFORM='wayland',QT_QUICK_BACKEND='software',QT_WAYLAND_DISABLE_WINDOWDECORATION='1')
    sway_config = root/'sway.conf'
    sway_config.write_text('output HEADLESS-1 mode 1280x900\nseat * fallback true\nfocus_follows_mouse no\n')
    qml = root/'shell.qml'
    qml.write_text('''import QtQuick
import Quickshell
import "components" as V
ShellRoot {
  V.VoxtypeCli { id: client; manifest: ({__sourceDir: '''+json.dumps(str(root))+'''}) }
  Connections {
    target: client
    ignoreUnknownSignals: true
    function onHistoryReply(token, ok, value, message) {
      console.log("HISTORY_TRANSPORT_REPLY:" + JSON.stringify({token:token,ok:ok,value:value,message:message}))
      Qt.quit()
    }
  }
  Component.onCompleted: Qt.callLater(function() {
    if (typeof client.historyRequest !== "function") {
      console.error("HISTORY_TRANSPORT_MISSING")
      Qt.quit()
      return
    }
    client.historyRequest("private-transport", ["list"])
  })
  Timer { interval: 10000; running: true; onTriggered: { console.error("HISTORY_TRANSPORT_TIMEOUT"); Qt.quit() } }
}
''')
    if case == 'window':
        qml.write_text('''import QtQuick
import Quickshell
import "components" as V
ShellRoot {
  V.VoxtypeCli { id: client; manifest: ({__sourceDir: '''+json.dumps(str(root))+'''}) }
  Loader {
    id: windowLoader
    source: "components/HistoryWindow.qml"
    onLoaded: { item.client = client; item.visible = true; item.refresh() }
  }
  Timer {
    interval: 100; running: true; repeat: true
    onTriggered: {
      if (windowLoader.status === Loader.Error) {
        console.error("HISTORY_WINDOW_MISSING")
        stop()
        Qt.quit()
      } else if (windowLoader.item && windowLoader.item.entryCount === 1) {
        console.log("HISTORY_WINDOW_READY:" + JSON.stringify({visible:windowLoader.item.visible, width:windowLoader.item.width, height:windowLoader.item.height}))
        stop()
      }
    }
  }
}
''')
    with (root/'sway.stdout').open('xb') as stdout, (root/'sway.stderr').open('xb') as stderr:
        compositor = subprocess.Popen(['sway','-c',str(sway_config)],env=env,stdin=subprocess.DEVNULL,stdout=stdout,stderr=stderr)
        try:
            deadline = time.monotonic()+15
            while True:
                sockets = [p for p in runtime.glob('wayland-*') if stat.S_ISSOCK(p.lstat().st_mode)]
                if sockets:
                    break
                assert compositor.poll() is None, (root/'sway.stderr').read_text()
                assert time.monotonic()<deadline, 'Private headless compositor did not become ready'
                time.sleep(0.05)
            env['WAYLAND_DISPLAY'] = sockets[0].name
            if case == 'window':
                deadline = time.monotonic()+10
                while not list(runtime.glob('sway-ipc*.sock')):
                    assert compositor.poll() is None
                    assert time.monotonic()<deadline
                    time.sleep(0.05)
                env['SWAYSOCK'] = str(next(runtime.glob('sway-ipc*.sock')))
                child = subprocess.Popen(['quickshell','-p',str(qml)],env=env,stdin=subprocess.DEVNULL,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,bufsize=0)
                assert child.stdout is not None
                try:
                    output = b''
                    deadline = time.monotonic()+20
                    while b'HISTORY_WINDOW_READY:' not in output and b'HISTORY_WINDOW_MISSING' not in output:
                        if child.poll() is not None:
                            break
                        assert time.monotonic()<deadline, output.decode(errors='replace')
                        if select.select([child.stdout],[],[],0.1)[0]:
                            output += os.read(child.stdout.fileno(),65536)
                    print(output.decode(errors='replace'))
                    assert b'HISTORY_WINDOW_MISSING' not in output, 'No real normal-window history host'
                    assert b'HISTORY_WINDOW_READY:' in output, 'History window did not load its real CLI data'
                    tree_result = subprocess.run(['swaymsg','-t','get_tree','-r'],env=env,capture_output=True,text=True,timeout=5)
                    assert tree_result.returncode==0, tree_result.stderr
                    def nodes(node):
                        yield node
                        for child_node in node.get('nodes',[])+node.get('floating_nodes',[]):
                            yield from nodes(child_node)
                    window_node = next((n for n in nodes(json.loads(tree_result.stdout)) if n.get('name')=='Voxtype History'),None)
                    assert window_node and window_node.get('shell')=='xdg_shell', window_node
                    assert window_node['rect']['width']>0 and window_node['rect']['height']>0
                    shot = subprocess.run(['grim','-o','HEADLESS-1','/work/out/history-quickshell-window.png'],env=env,capture_output=True,text=True,timeout=5)
                    assert shot.returncode==0, shot.stderr
                    print(json.dumps({'case':'real-quickshell-history-window','entries':1,'surface':window_node,'capture':'/work/out/history-quickshell-window.png','boundary':'real xdg-shell window on private headless Sway, not installed Omarchy or physical desktop'}))
                finally:
                    child.terminate()
                    try:
                        child.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        child.kill()
                        child.wait(timeout=5)
                raise SystemExit(0)
            result = subprocess.run(['quickshell','-p',str(qml)],env=env,stdin=subprocess.DEVNULL,capture_output=True,text=True,timeout=25)
            output = result.stdout+'\n'+result.stderr
            print(output)
            assert 'HISTORY_TRANSPORT_MISSING' not in output, 'Canonical Quickshell client has no history request transport'
            assert result.returncode==0, result.returncode
            marker = 'HISTORY_TRANSPORT_REPLY:'
            reply = next((json.loads(line.split(marker,1)[1]) for line in output.splitlines() if marker in line),None)
            assert reply and reply['ok'], reply
            assert reply['token']=='private-transport'
            assert reply['value']['enabled'] is True
            assert [e['id'] for e in reply['value']['entries']]==[entry_id]
            print(json.dumps({'case':'real-quickshell-history-transport','entries':1,'boundary':'private headless compositor, real CLI and Quickshell; not installed Omarchy'}))
        finally:
            compositor.terminate()
            try:
                compositor.wait(timeout=5)
            except subprocess.TimeoutExpired:
                compositor.kill()
                compositor.wait(timeout=5)
