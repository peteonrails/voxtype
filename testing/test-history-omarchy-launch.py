"""Staged actual Panel/IPC/CLI route in private Sway.
Reduced host: real PluginShellApi plus asynchronous Loader/manifest injection;
registry callbacks are local fixture callbacks, not installed-shell acceptance.
"""
import datetime,json,os,shutil,stat,subprocess,tempfile,time
from pathlib import Path
assert os.environ.get('HOME')=='/work/home'
assert not any(os.environ.get(k) for k in ['DISPLAY','WAYLAND_DISPLAY','SWAYSOCK','HYPRLAND_INSTANCE_SIGNATURE'])
assert bool(os.environ.get('DBUS_SESSION_BUS_ADDRESS')) == (os.environ.get('VOX_HISTORY_OMARCHY_CASE')=='meeting-link')
os.umask(0o077)
source=Path(__file__).resolve().parents[1]
exe=Path(os.environ.get('VOXTYPE_HISTORY_TEST_BIN','/work/target/debug/voxtype'))
shell=Path('/opt/history-omarchy'); assert (shell/'Commons/Style.qml').is_file()
qs: str=shutil.which('qs') or ''; assert qs
case=os.environ.get('VOX_HISTORY_OMARCHY_CASE','panel')
assert case in ['panel','types','async-control','panel-sync','meeting-link']
with tempfile.TemporaryDirectory(dir='/work/scratch',prefix='history-omarchy-launch-') as name:
 root=Path(name); runtime=root/'runtime'; runtime.mkdir(mode=0o700)
 config=root/'config/voxtype/config.toml'; config.parent.mkdir(parents=True)
 config.write_text('[hotkey]\nenabled=false\n[history]\nenabled=true\n')
 caller_a=root/'caller-a'; caller_b=root/'caller-b'
 caller_a.mkdir(); caller_b.mkdir()
 config_a=caller_a/'config.toml'; config_b=caller_b/'config.toml'
 config_a.write_text('[history]\nenabled=true\npaused=false\n')
 config_b.write_text('[history]\nenabled=false\npaused=false\n')
 original_a=config_a.read_bytes(); original_default=config.read_bytes()
 now=datetime.datetime.now().astimezone(); folder=root/'data/voxtype/history'; folder.mkdir(parents=True,mode=0o700)
 entry_id=now.strftime('%Y%m%d-%H%M%S')+'-abc123'
 (folder/(now.strftime('%Y-%m-%d')+'.jsonl')).write_text(json.dumps(dict(schema=1,id=entry_id,ts_start=now.isoformat(),ts_end=now.isoformat(),raw_text='synthetic integrated launch',text='Synthetic integrated launch.',engine='whisper',model='synthetic.gguf',delivered=True,outcome='delivered'))+'\n')
 env=dict(os.environ,HOME=str(root),XDG_CONFIG_HOME=str(root/'config'),XDG_DATA_HOME=str(root/'data'),XDG_RUNTIME_DIR=str(runtime),WLR_BACKENDS='headless',WLR_RENDERER='pixman',WLR_LIBINPUT_NO_DEVICES='1',WLR_HEADLESS_OUTPUTS='1',QT_QPA_PLATFORM='wayland',QT_QUICK_BACKEND='software',QT_WAYLAND_DISABLE_WINDOWDECORATION='1',QS_DISABLE_FILE_WATCHER='1',QS_NO_RELOAD_POPUP='1',QML_DISABLE_DISK_CACHE='1')
 links=None
 if case=='meeting-link':
  from history_meeting_links import MeetingLinks
  env.update(QT_LINUX_ACCESSIBILITY_ALWAYS_ON='1',QT_ACCESSIBILITY='1',GDK_BACKEND='wayland',GSK_RENDERER='cairo',GTK_A11Y='atspi')
  links=MeetingLinks(root,env,exe,{'A':config_a,'B':config_b,'D':config})
 staged=root/'plugins'
 result=subprocess.run(['bash','scripts/stage-omarchy-plugin.sh',str(source/'omarchy-plugin'),str(staged)],env=env,capture_output=True,text=True,timeout=15)
 assert result.returncode==0,result.stdout+result.stderr
 plugin=staged/'io.voxtype.settings'; manifest=json.loads((plugin/'manifest.json').read_text())
 manifest['__sourceDir']=str(plugin)
 panel_entry=plugin/manifest['entryPoints']['panel']
 if case=='async-control':
  panel_entry=root/'async-control.qml'
  panel_entry.write_text('import QtQuick\nItem { property var shell: null; property var manifest: null }\n')

 (plugin/'dev.json').write_text(json.dumps({'voxtypeBin':str(links.client_bin if links is not None else exe)}))
 for child in shell.iterdir():
  if child.is_dir() and not (root/child.name).exists(): shutil.copytree(child,root/child.name,symlinks=True)
 shutil.copyfile(shell/'services/PluginShellApi.qml',root/'HostApi.qml')
 harness=root/'shell.qml'
 harness.write_text('''import QtQuick
import Quickshell
import Quickshell.Io
import qs.Commons
import qs.Commons as Commons
ShellRoot {
 id: fixture
 property bool ready: false
 property var publicManifest: MANIFEST

 // The real Omarchy host already has mapped shell surfaces when it incubates
 // asynchronous plugins. Preserve that precondition in this reduced host.
 PanelWindow {
  id: hostSurface
  visible: true
  anchors { bottom: true; left: true }
  implicitWidth: 320 + Style.cornerRadius
  implicitHeight: 24
  exclusiveZone: 0
  color: "#101010"
  Rectangle {
   anchors.fill: parent
   color: "#101010"
   NumberAnimation on opacity { from: 0.99; to: 1; duration: 1000; loops: Animation.Infinite; running: true }
  }
 }
 HostApi {
  id: api
  pluginId: "io.voxtype.settings"
  _summon: function(id, payload) { if (id !== pluginId || !panel.item) return false; panel.item.open(payload); return true }
  _hide: function(id) { if (id !== pluginId || !panel.item) return false; panel.item.close(); return true }
  _isOpen: function(id) { return id === pluginId && panel.item && panel.item.opened }
 }
 Loader {
  id: panel
  asynchronous: ASYNC_FLAG
  source: PANEL_URL
  onStatusChanged: console.log("ACTUAL_PANEL_LOADER_STATUS",status)
  onLoaded: { item.shell=api; item.manifest=fixture.publicManifest; fixture.ready=true }
 }
 function historyContent(): var {
   var h=panel.item ? panel.item.activeHistoryHost : null
   var children=h && h.window && h.window.contentItem ? h.window.contentItem.children : []
   for (var i=0;i<children.length;++i)
    if (children[i].historyEnabled !== undefined && typeof children[i].changeState === "function") return children[i]
   return null
 }
 IpcHandler {
  target: "fixture"
  function activate(name: string): string {
   function find(item) {
    if (item.objectName === name) return item
    var children=item.children || []
    for (var i=0;i<children.length;++i) { var found=find(children[i]); if(found) return found }
    return null
   }
   var item=fixture.historyContent()
   var button=item ? find(item) : null
   if (!button || !button.visible || !button.enabled) return "control unavailable"
   button.clicked()
   return "activated actual control"
  }
  function status(): string {
   var p=panel.item
   var h=p ? p.activeHistoryHost : null
   var c=fixture.historyContent()
   return JSON.stringify({ready:fixture.ready,hostVisible:hostSurface.visible,view:p ? p.view : "",opened:p ? p.opened : false,count:h && h.window ? h.window.entryCount : -1,config:h ? h.configPath : null,state:c ? c.stateText : null,hostCount:p && p.historyHosts ? Object.keys(p.historyHosts).length : 0})
  }
 }
}
'''.replace('MANIFEST',json.dumps(manifest)).replace('PANEL_URL',json.dumps(panel_entry.as_uri())).replace('ASYNC_FLAG','false' if case=='panel-sync' else 'true'))
 tools=root/'tools'; tools.mkdir(mode=0o700)
 wrapper=tools/'omarchy-shell'
 wrapper.write_text('#!/usr/bin/python3\nimport os,sys\nos.execv('+repr(qs)+',['+repr(qs)+',"ipc","-p",'+repr(str(harness))+',"call",*sys.argv[1:]])\n'); wrapper.chmod(0o700)
 env['PATH']=str(tools)+':'+env['PATH']
 sway_config=root/'sway.conf'; sway_config.write_text('output HEADLESS-1 mode 1280x900\nxwayland disable\nseat * fallback true\nfocus_follows_mouse no\n')
 compositor=None; panel_process=None
 def wait(check,message,timeout=15):
  deadline=time.monotonic()+timeout
  while True:
   value=check()
   if value: return value
   assert compositor is not None and compositor.poll() is None,message+'; private compositor exited'
   assert panel_process is None or panel_process.poll() is None,message+'; panel exited'
   assert time.monotonic()<deadline,message+'; last actual status='+json.dumps(status())
   time.sleep(.05)
 def status():
  result=subprocess.run([qs,'ipc','-p',str(harness),'call','fixture','status'],env=env,capture_output=True,text=True,timeout=3)
  if result.returncode: return {}
  return json.loads(result.stdout)
 def mapped():
  result=subprocess.run(['swaymsg','-t','get_tree','-r'],env=env,capture_output=True,text=True,timeout=5)
  assert result.returncode==0,result.stderr
  def nodes(node):
   yield node
   for child in node.get('nodes',[])+node.get('floating_nodes',[]): yield from nodes(child)
  return [n for n in nodes(json.loads(result.stdout)) if n.get('name')=='VoxType History']
 with (root/'sway.stdout').open('xb') as sway_out,(root/'sway.stderr').open('xb') as sway_err,(root/'panel.stdout').open('xb') as panel_out,(root/'panel.stderr').open('xb') as panel_err:
  try:
   compositor=subprocess.Popen(['sway','-c',str(sway_config)],env=env,stdin=subprocess.DEVNULL,stdout=sway_out,stderr=sway_err)
   def sockets_ready():
    sockets=[p for p in runtime.glob('wayland-*') if stat.S_ISSOCK(p.lstat().st_mode)]
    ipc=list(runtime.glob('sway-ipc*.sock'))
    return (sockets[0],ipc[0]) if sockets and ipc else None
   wayland,ipc=wait(sockets_ready,'Private compositor did not start')
   env.update(WAYLAND_DISPLAY=wayland.name,SWAYSOCK=str(ipc))
   if case=='types':
    definitions=[{'scope':label,'url':path.as_uri()} for label,path in [('source-panel',source/'omarchy-plugin/Panel.qml'),('staged-panel',plugin/manifest['entryPoints']['panel'])]]
    diagnostic=root/'types.qml'
    template='''import QtQuick
import Quickshell
ShellRoot {
 id: comparison
 property var definitions: DEFINITIONS
 property var records: []
 property int position: 0
 property var pending: null
 function finish() {
  var passed=records.length === definitions.length
  for (var i=0;i<records.length;++i) if (records[i].status !== Component.Ready) passed=false
  console.log("OMARCHY_TYPE_COMPARISON",JSON.stringify(records))
  console.log(passed ? "TYPE_COMPARISON_PASS" : "TYPE_COMPARISON_FAIL")
  Qt.callLater(Qt.quit)
 }
 function completed() {
  if (!pending || pending.status === Component.Loading) return
  console.log("ACTUAL_COMPONENT",definitions[position].scope,pending.status,pending.errorString())
  records.push({scope:definitions[position].scope,status:pending.status,error:pending.errorString()})
  pending.statusChanged.disconnect(completed)
  pending=null
  position++
  Qt.callLater(next)
 }
 function next() {
  if (position === definitions.length) { finish(); return }
  console.log("ACTUAL_COMPONENT_BEGIN",definitions[position].scope,definitions[position].url)
  pending=Qt.createComponent(definitions[position].url,Component.PreferSynchronous)
  console.log("ACTUAL_COMPONENT_CREATED",definitions[position].scope,pending.status,pending.errorString())
  pending.statusChanged.connect(completed)
  completed()
 }
 Component.onCompleted: comparison.next()
 Timer { interval: 20000; running: true; onTriggered: comparison.finish() }
}
'''
    passed=[]
    for definition in definitions:
     diagnostic.write_text(template.replace('DEFINITIONS',json.dumps([definition])))
     try: compared=subprocess.run([qs,'-p',str(diagnostic)],env=env,capture_output=True,text=True,timeout=30)
     except subprocess.TimeoutExpired as error:
      print('NATIVE_TYPE_TIMEOUT_STDOUT',definition['scope'],error.stdout)
      print('NATIVE_TYPE_TIMEOUT_STDERR',definition['scope'],error.stderr)
      passed.append(False)
      continue
     print('NATIVE_TYPE_COMPARISON',definition['scope'],compared.returncode,compared.stdout,compared.stderr)
     passed.append(compared.returncode==0 and 'TYPE_COMPARISON_PASS' in compared.stdout)
    assert all(passed), 'Isolated native source/staged component comparison failed; not a history-route result'
    raise SystemExit(0)
   panel_process=subprocess.Popen([qs,'-n','-p',str(harness)],env=env,stdin=subprocess.DEVNULL,stdout=panel_out,stderr=panel_err)
   wait(lambda:status().get('ready'),'Actual packaged history entry failed to load')
   if case=='async-control':
    print(json.dumps({'case':'async-toolkit-control-only','settled':True,'not_a_voxtype_panel_or_application_route':True}))
    raise SystemExit(0)
   if case=='meeting-link':
    def linked_launch(label):
     result=subprocess.run([str(exe),*(['--config','./config.toml'] if label!='D' else []),'history','view'],cwd=(caller_a if label=='A' else caller_b) if label!='D' else root,env=env,capture_output=True,text=True,timeout=10)
     print('ACTUAL_PACKAGED_OMARCHY_HISTORY_LINK_LAUNCH',label,result.returncode,result.stdout,result.stderr,flush=True)
     assert result.returncode==0,result.stderr
     wait(lambda:len(mapped())==1 and status().get('config')==('' if label=='D' else str(config_a if label=='A' else config_b)),'Actual public launcher must select correct plugin config and normal window')
    assert links is not None
    links.exercise(linked_launch,'omarchy')
    raise SystemExit(0)
   probe=subprocess.run([str(wrapper),'io.voxtype.settings','historyConfig',str(config_a)],env=env,capture_output=True,text=True,timeout=5)
   print('ACTUAL_HISTORY_IPC',probe.returncode,probe.stdout,probe.stderr)
   assert probe.returncode==0 and probe.stdout.strip()=='ok','Actual packaged Panel must expose the canonical historyConfig IPC route'
   wait(lambda:status().get('view')=='history' and status().get('count')==1,'Integrated history did not read the synthetic authoritative entry')
   wait(lambda:len(mapped())==1,'Integrated history did not map a normal window')
   first=mapped()[0]; assert first.get('shell')=='xdg_shell',first
   for _ in range(2):
    opened=subprocess.run([str(exe),'--config','config.toml','history','view'],cwd=caller_a,env=env,capture_output=True,text=True,timeout=10)
    print('CANONICAL_HISTORY_OPEN',opened.returncode,opened.stdout,opened.stderr)
    assert opened.returncode==0 and opened.stdout.strip()=='Opened the history window.'
    assert status()['view']=='history' and status()['count']==1
    assert len(mapped())==1 and mapped()[0]['id']==first['id'],'Integrated canonical opens must be idempotent'
   shot=subprocess.run(['grim','-o','HEADLESS-1','/work/out/history-omarchy-staged.png'],env=env,capture_output=True,text=True,timeout=5)
   assert shot.returncode==0,shot.stderr
   opened_b=subprocess.run([str(exe),'--config','./config.toml','history','view'],cwd=caller_b,env=env,capture_output=True,text=True,timeout=10)
   assert opened_b.returncode==0,opened_b.stdout+opened_b.stderr
   wait(lambda:status().get('config')==str(config_b) and status().get('state')=='History off','Actual integrated forwarded caller B must retain its own config and state')
   def control(name):
    result=subprocess.run([qs,'ipc','-p',str(harness),'call','fixture','activate',name],env=env,capture_output=True,text=True,timeout=5)
    assert result.returncode==0 and result.stdout.strip()=='activated actual control',result.stdout+result.stderr
   def config_state(path):
    result=subprocess.run([str(exe),'--config',str(path),'history','--json','status'],env=env,capture_output=True,text=True,timeout=5)
    assert result.returncode==0,result.stderr
    return json.loads(result.stdout)
   control('history-enabled')
   wait(lambda:config_state(config_b)['enabled'] and status().get('state')=='History on','Actual integrated enable must save and reload B')
   control('history-pause')
   wait(lambda:config_state(config_b)['paused'] and status().get('state')=='History paused','Actual integrated pause must save and reload B')
   assert config_a.read_bytes()==original_a and config.read_bytes()==original_default,'Integrated B actions must not mutate caller A or absent/default config'
   forwarded_shot=subprocess.run(['grim','-o','HEADLESS-1','/work/out/history-omarchy-forwarded.png'],env=env,capture_output=True,text=True,timeout=5)
   assert forwarded_shot.returncode==0,forwarded_shot.stderr
   reopened_a=subprocess.run([str(exe),'--config','config.toml','history','view'],cwd=caller_a,env=env,capture_output=True,text=True,timeout=10)
   assert reopened_a.returncode==0,reopened_a.stdout+reopened_a.stderr
   wait(lambda:status().get('config')==str(config_a) and status().get('state')=='History on','Reopening A must reuse its immutable client and own state')
   assert status()['hostCount']==2 and len(mapped())==1,'Exactly two retained clients, one current visible history window'
   print(json.dumps({'case':'packaged-omarchy-history-panel-IPC-canonical-launch','loader_mode':case,'manifest_entry':manifest['entryPoints']['panel'],'status':status(),'surface':mapped()[0],'scope':'actual staged Panel and public CLI/native IPC, real PluginShellApi; reduced local Loader/summon callbacks. panel-sync is a synchronous construction contract only, NOT async/full installed shell acceptance; default panel remains asynchronous.'}))
  finally:
   if panel_process is not None and panel_process.poll() is None:
    process_root=Path('/proc')/str(panel_process.pid)
    observations=[]
    for task in sorted((process_root/'task').iterdir()):
     try:
      observations.append({'tid':task.name,'comm':(task/'comm').read_text().strip(),'wait_channel':(task/'wchan').read_text().strip(),'syscall':(task/'syscall').read_text().strip()})
     except FileNotFoundError: pass
     except PermissionError as error:
      observations.append({'tid':task.name,'permission_denied':str(error),'operation_stopped':True})
      break
    descriptors={}
    for descriptor in (process_root/'fd').iterdir():
     try: descriptors[descriptor.name]=os.readlink(descriptor)
     except FileNotFoundError: pass
     except PermissionError as error:
      descriptors['permission_denied']=str(error)
      break
    print('OWNED_QML_PROCESS_OBSERVATION',json.dumps({'pid':panel_process.pid,'threads':observations,'descriptors':descriptors}))
   for process in [panel_process,compositor]:
    if process is not None and process.poll() is None:
     process.terminate()
     try: process.wait(timeout=5)
     except subprocess.TimeoutExpired: process.kill(); process.wait(timeout=5)
   panel_out.flush(); panel_err.flush(); sway_err.flush()
   print('ACTUAL_PANEL_STDOUT',(root/'panel.stdout').read_text(errors='replace'))
   print('ACTUAL_PANEL_STDERR',(root/'panel.stderr').read_text(errors='replace'))
   print('PRIVATE_SWAY_STDERR',(root/'sway.stderr').read_text(errors='replace'))
