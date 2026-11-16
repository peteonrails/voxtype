"""Actual recording-free daemon tray -> canonical history on private Sway/DBus.
This is a minimal real SNI/QsMenuOpener host, not an installed desktop claim.
Only history actions are activated; no audio/input devices or real clipboard.
"""
import datetime,hashlib,json,os,shutil,signal,stat,subprocess,tempfile,time,tomllib
from pathlib import Path
import pyatspi
from gi.repository import GLib
assert os.environ.get('HOME')=='/work/home' and os.environ.get('DBUS_SESSION_BUS_ADDRESS')
assert not any(os.environ.get(k) for k in ['DISPLAY','WAYLAND_DISPLAY','SWAYSOCK','HYPRLAND_INSTANCE_SIGNATURE'])
os.umask(0o077)
source=Path(__file__).resolve().parents[1];exe=Path(os.environ.get('VOXTYPE_HISTORY_TEST_BIN','/work/target/debug/voxtype'))
case=os.environ.get('VOX_HISTORY_TRAY_CASE','explicit');assert case in ['explicit','legacy']
with tempfile.TemporaryDirectory(dir='/work/scratch',prefix='history-native-tray-') as name:
 root=Path(name);runtime=root/'runtime';runtime.mkdir(mode=0o700)
 default=root/'config/voxtype/config.toml';default.parent.mkdir(parents=True);default.write_text('# Untouched default sentinel\n[history]\nenabled=false\n')
 config=root/'chosen.toml'
 settings='# Chosen private config\n[hotkey]\nenabled=false\n[whisper]\non_demand_loading=true\n[osd]\nenabled=false\n[tray]\nenabled=true\n[meeting]\nenabled=false\noffer_enable=false\n[meeting.detect]\nenabled=false\n'
 if case=='explicit':settings+='[history]\nenabled=false\npaused=true\nrecord_window_title=false\nretention_days=0\nexclude_apps=["KeePassXC","Bitwarden","1Password","Proton Pass","private-sensitive"]\n'
 config.write_text(settings);untouched=default.read_bytes()
 data=root/'data/voxtype/history';data.mkdir(parents=True,mode=0o700);now=datetime.datetime.now().astimezone()
 text='Private canonical tray recovery.'
 (data/(now.strftime('%Y-%m-%d')+'.jsonl')).write_text(json.dumps(dict(schema=1,id=now.strftime('%Y%m%d-%H%M%S')+'-abc123',ts_start=now.isoformat(),ts_end=now.isoformat(),raw_text='private canonical tray recovery',text=text,engine='whisper',model='synthetic.gguf',delivered=True,outcome='delivered'))+'\n')
 tools=root/'tools';tools.mkdir();sink=root/'copied.bin'
 copy=tools/'wl-copy';copy.write_text('#!/usr/bin/python3\nimport sys\nfrom pathlib import Path\nPath('+repr(str(sink))+').write_bytes(sys.stdin.buffer.read())\n');copy.chmod(0o500)
 env=dict(os.environ,HOME=str(root),XDG_CONFIG_HOME=str(root/'config'),XDG_DATA_HOME=str(root/'data'),XDG_RUNTIME_DIR=str(runtime),PATH=str(tools)+os.pathsep+os.environ['PATH'],WLR_BACKENDS='headless',WLR_RENDERER='pixman',WLR_LIBINPUT_NO_DEVICES='1',WLR_HEADLESS_OUTPUTS='1',QT_QPA_PLATFORM='wayland',QT_QUICK_BACKEND='software',QT_WAYLAND_DISABLE_WINDOWDECORATION='1',QT_LINUX_ACCESSIBILITY_ALWAYS_ON='1',QT_ACCESSIBILITY='1',QML_DISABLE_DISK_CACHE='1',QS_DISABLE_FILE_WATCHER='1',QS_NO_RELOAD_POPUP='1')
 target=root/'data/voxtype/quickshell'
 installed=subprocess.run([str(exe),'setup','quickshell','--source',str(source/'quickshell'),'--target',str(target),'--skip-bridge'],env=env,capture_output=True,text=True,timeout=20)
 assert installed.returncode==0,installed.stdout+installed.stderr
 env['VOXTYPE_OSD_QML_PATH']=str(target)
 (root/'sway.conf').write_text('output HEADLESS-1 mode 1280x1000\nseat * fallback true\nfocus_follows_mouse no\n')
 qml='''import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import QtQuick.Window
import Quickshell
import Quickshell.Widgets
import Quickshell.Services.SystemTray
Window {
 id: root; visible: true; width: 500; height: 950; color: "#151820"; title: "Private native history tray"
 property var item: null
 property bool menuOpen: false
 QsMenuOpener { id: opener; menu: root.item ? root.item.menu : null }
 ColumnLayout {
  anchors.fill: parent; anchors.margins: 16
  Label { text: root.item ? root.item.tooltipTitle : "Waiting for native tray"; color: "white" }
  Button { text: "VoxType tray"; enabled: root.item !== null; onClicked: root.menuOpen = !root.menuOpen
   IconImage { anchors.right: parent.right; width: 28; height: 28; source: root.item ? root.item.icon : "" }
  }
  Label { text: root.item ? "Native onlyMenu: " + root.item.onlyMenu : "No native item"; color: "white" }
  ColumnLayout {
   visible: root.menuOpen
   Repeater { model: opener.children.values
    Button {
     required property var modelData
     text: modelData.text || ""; visible: text !== ""; enabled: modelData.enabled !== false
     onClicked: modelData.triggered()
    }
   }
  }
  Item { Layout.fillHeight: true }
 }
 Timer { interval: 100; running: true; repeat: true; onTriggered: {
  var values = SystemTray.items.values
  for (var i=0;i<values.length;++i) if (values[i].id === "voxtype") root.item=values[i]
 } }
}'''
 (root/'tray.qml').write_text(qml)
 processes=[];files=[]
 def start(argv,label):
  log=(root/(label+'.log')).open('w');files.append(log);p=subprocess.Popen(argv,env=env,stdin=subprocess.DEVNULL,stdout=log,stderr=subprocess.STDOUT);processes.append(p);return p
 def walk(node,depth=0):
  if depth>28:return
  try:count=node.childCount;node.name
  except GLib.Error:return
  yield node
  for i in range(count):
   try:child=node.getChildAtIndex(i)
   except GLib.Error:continue
   if child is not None:yield from walk(child,depth+1)
 def allnodes():return list(walk(pyatspi.Registry.getDesktop(0)))
 def inside(title):
  frame=next((n for n in allnodes() if n.getRoleName()=='frame' and n.name==title and n.getState().contains(pyatspi.STATE_SHOWING)),None)
  return list(walk(frame)) if frame is not None else []
 def wait(fn,message,seconds=15):
  deadline=time.monotonic()+seconds
  while True:
   result=fn()
   if result:return result
   assert time.monotonic()<deadline,(message,[(n.name,n.getRoleName()) for n in allnodes()])
   time.sleep(.05)
 def press(title,label):
  button=wait(lambda:next((n for n in inside(title) if n.name==label and n.getRoleName() in ['button','push button'] and n.getState().contains(pyatspi.STATE_ENABLED)),None),'Missing enabled actual '+label)
  assert button.queryAction().doAction(0)
 def read():return tomllib.loads(config.read_text()).get('history',{})
 def shot(label):subprocess.run(['grim','-o','HEADLESS-1','/work/out/history-tray-'+case+'-'+label+'.png'],env=env,check=True,timeout=5)
 try:
  compositor=start(['sway','-c',str(root/'sway.conf')],'sway')
  def ready():
   sockets=[p for p in runtime.glob('wayland-*') if stat.S_ISSOCK(p.lstat().st_mode)];ipc=list(runtime.glob('sway-ipc*.sock'))
   if sockets and ipc:env.update(WAYLAND_DISPLAY=sockets[0].name,SWAYSOCK=str(ipc[0]));return True
  wait(ready,'Private compositor did not bind')
  host=start(['qs','-n','-p',str(root/'tray.qml')],'tray-host')
  daemon=start([str(exe),'--config',str(config)],'daemon')
  wait(lambda:any(n.name=='Native onlyMenu: true' for n in inside('Private native history tray')),'Actual daemon native tray missing')
  assert daemon.poll() is None
  digest=hashlib.sha256(Path('/proc',str(daemon.pid),'exe').read_bytes()).hexdigest();assert digest==hashlib.sha256(exe.read_bytes()).hexdigest()
  press('Private native history tray','VoxType tray')
  wait(lambda:any(n.name=='Enable History' for n in inside('Private native history tray')),'Off menu must offer Enable History');shot('off')
  press('Private native history tray','Enable History');wait(lambda:read().get('enabled') is True,'Actual tray action did not enable chosen private file')
  expected='Resume History (paused)' if case=='explicit' else 'Pause History'
  wait(lambda:any(n.name==expected for n in inside('Private native history tray')),'Native menu did not reload enabled/paused state')
  if case=='explicit':
   assert read()['paused'] is True and read()['exclude_apps']==['KeePassXC','Bitwarden','1Password','Proton Pass','private-sensitive'] and read()['retention_days']==0
   press('Private native history tray','Resume History (paused)');wait(lambda:read().get('paused') is False,'Resume must persist chosen file')
  press('Private native history tray','Pause History');wait(lambda:read().get('paused') is True,'Pause must persist chosen file')
  wait(lambda:any(n.name=='Resume History (paused)' for n in inside('Private native history tray')),'Pause must be visible in actual menu');shot('paused')
  press('Private native history tray','Resume History (paused)');wait(lambda:read().get('paused') is False,'Resume must persist chosen file')
  press('Private native history tray','Copy last transcript');wait(lambda:sink.exists(),'Canonical copy action missing');assert sink.read_bytes()==text.encode()
  press('Private native history tray','History')
  wait(lambda:any(n.name==text for n in inside('VoxType History')),'Actual native tray must open canonical installed History window with saved content')
  wait(lambda:any(n.name=='History on' for n in inside('VoxType History')),'History window must use enabled chosen file, not off default');shot('history')
  assert default.read_bytes()==untouched and daemon.poll() is None
  descriptors=[os.readlink(p) for p in Path('/proc',str(daemon.pid),'fd').iterdir() if p.exists()]
  assert not any(p.startswith(('/dev/snd','/dev/input')) for p in descriptors)
  assert 'Recording started' not in (root/'daemon.log').read_text()
  result=dict(case=case,passed=True,native_pid=daemon.pid,native_sha256=digest,chosen_config=str(config),default_unchanged=True,menu_enable_pause_resume=True,copy_bytes_exact=True,actual_installed_history_frame=True,no_recording_or_device_access=True,boundary='real idle daemon+SNI+QsMenuOpener rendered private host; not installed Omarchy/physical desktop or inference')
  Path('/work/out/history-tray-'+case+'.json').write_text(json.dumps(result,indent=2));print('ACTUAL_NATIVE_HISTORY_TRAY',json.dumps(result),flush=True)
 finally:
  if env.get('WAYLAND_DISPLAY'):
   try:shot('final')
   except Exception:pass
  for p in reversed(processes):
   if p.poll() is None:p.terminate()
   try:p.wait(timeout=8)
   except subprocess.TimeoutExpired:p.kill();p.wait(timeout=5)
  for f in files:f.close()
  print('PRIVATE_TRAY_LOGS',json.dumps({p.name:p.read_text()[-10000:] for p in root.glob('*.log')}),flush=True)
