"""Real staged History settings controls in private Sway, real canonical CLI.
Only read-only aliases are added to the staged Panel for QtTest native events.
The transparent native observer refuses the unrelated info-devices probe with
an explicit nonzero exit. It never fabricates schema/write/status responses.
"""
import hashlib,json,os,select,shutil,socket,stat,subprocess,tempfile,time,tomllib
from pathlib import Path
assert os.getuid()==65532 and os.environ['HOME']=='/work/home'
assert not any(os.environ.get(k) for k in ['DISPLAY','WAYLAND_DISPLAY','SWAYSOCK','HYPRLAND_INSTANCE_SIGNATURE','DBUS_SESSION_BUS_ADDRESS'])
os.umask(0o077)
source=Path(__file__).resolve().parents[1];exe=Path(os.environ['VOXTYPE_HISTORY_TEST_BIN']);native_sha=hashlib.sha256(exe.read_bytes()).hexdigest()
case=os.environ.get('VOX_HISTORY_SETTINGS_CASE','explicit');assert case in ['explicit','default']
steps=[];processes=[];logs=[];passed=False;failure=''
with tempfile.TemporaryDirectory(prefix='history-settings-',dir='/work/scratch') as name:
 root=Path(name);runtime=root/'runtime';runtime.mkdir(mode=0o700)
 default=root/'config/voxtype/config.toml';default.parent.mkdir(parents=True,mode=0o700)
 explicit=root/'chosen.toml';chosen,sentinel=(explicit,default) if case=='explicit' else (default,explicit)
 chosen.write_text('# Private old settings fixture\n[whisper]\nmodel="private-missing-model" # preserve model\n[text]\nfilter_filler_words=false # preserve unrelated\n')
 sentinel.write_text('# Unselected private sentinel\n[history]\nenabled=true\npaused=true\n');untouched=sentinel.read_bytes()
 assert chosen.stat().st_mode&0o777==0o600 and not Path(str(chosen)+'.tmp').exists()
 # Private fixture initialization must not mask replacement-mode bugs.
 os.umask(0o022)
 audio=socket.socket(socket.AF_UNIX);audio.bind(str(root/'audio.sock'));audio.listen(1);audio.setblocking(False)
 env=dict(os.environ,HOME=str(root),XDG_CONFIG_HOME=str(root/'config'),XDG_DATA_HOME=str(root/'data'),XDG_RUNTIME_DIR=str(runtime),PULSE_SERVER='unix:'+str(root/'audio.sock'),WLR_BACKENDS='headless',WLR_RENDERER='pixman',WLR_LIBINPUT_NO_DEVICES='1',WLR_HEADLESS_OUTPUTS='1',QT_QPA_PLATFORM='wayland',QT_QUICK_BACKEND='software',QT_WAYLAND_DISABLE_WINDOWDECORATION='1',QS_DISABLE_FILE_WATCHER='1',QS_NO_RELOAD_POPUP='1',QML_DISABLE_DISK_CACHE='1')
 staged=root/'plugins';stage=subprocess.run(['bash','scripts/stage-omarchy-plugin.sh',str(source/'omarchy-plugin'),str(staged)],env=env,capture_output=True,text=True,timeout=20);assert stage.returncode==0,stage.stdout+stage.stderr
 plugin=staged/'io.voxtype.settings';manifest=json.loads((plugin/'manifest.json').read_text());manifest['__sourceDir']=str(plugin)
 entry=plugin/manifest['entryPoints']['panel'];original=entry.read_text();assert original.count('  id: root')==1
 entry.write_text(original.replace('  id: root','  id: root\n  readonly property alias formTest: form\n  readonly property alias cliTest: cli\n  readonly property alias renderTest: settingsCard.card',1))
 observer=root/'native-observer';journal=root/'native.jsonl'
 observer.write_text('''#!/usr/bin/python3
import base64,hashlib,json,os,subprocess,sys
from pathlib import Path
exe=Path(EXE);args=sys.argv[1:];prefix=PREFIX;want=HASH
assert hashlib.sha256(exe.read_bytes()).hexdigest()==want
record=dict(argv=[str(exe),*prefix,*args],native_sha256=want,uid=os.getuid())
if args[:2]==['info','devices']:
 record.update(blocked_unrelated_device_probe=True,rc=77)
 with Path(JOURNAL).open('a') as f:f.write(json.dumps(record)+'\\n')
 print('PRIVATE_FIXTURE: unrelated audio-device probe refused',file=sys.stderr);sys.exit(77)
with Path(JOURNAL).open('a') as f:f.write(json.dumps(dict(record,phase='started'))+'\\n')
try:p=subprocess.run(record['argv'],capture_output=True,timeout=15)
except subprocess.TimeoutExpired:
 with Path(JOURNAL).open('a') as f:f.write(json.dumps(dict(record,phase='actual_subprocess_timeout'))+'\\n')
 raise
record.update(rc=p.returncode,stdout_sha256=hashlib.sha256(p.stdout).hexdigest(),stderr_sha256=hashlib.sha256(p.stderr).hexdigest())
assert hashlib.sha256(exe.read_bytes()).hexdigest()==want
with Path(JOURNAL).open('a') as f:f.write(json.dumps(record)+'\\n')
sys.stdout.buffer.write(p.stdout);sys.stderr.buffer.write(p.stderr);sys.exit(p.returncode)
'''.replace('EXE',repr(str(exe))).replace('PREFIX',repr(['--config',str(chosen)] if case=='explicit' else [])).replace('HASH',repr(native_sha)).replace('JOURNAL',repr(str(journal))))
 observer.chmod(0o500);(plugin/'dev.json').write_text(json.dumps({'voxtypeBin':str(observer)}))
 shell=Path('/opt/history-omarchy');assert (shell/'Commons/Style.qml').is_file()
 for child in shell.iterdir():
  if child.is_dir() and not (root/child.name).exists():shutil.copytree(child,root/child.name,symlinks=True)
 shutil.copyfile(shell/'services/PluginShellApi.qml',root/'HostApi.qml')
 qml=root/'shell.qml';qml.write_text('''import QtQuick
import QtTest
import Quickshell
import Quickshell.Io
import qs.Commons
ShellRoot {
 id: fixture
 property var publicManifest: MANIFEST
 PanelWindow { visible:true; anchors { bottom:true;left:true } implicitWidth:330;implicitHeight:24;exclusiveZone:0;color:"#101010"
  Rectangle {anchors.fill:parent;color:"#101010";NumberAnimation on opacity {from:0.99;to:1;duration:1000;loops:Animation.Infinite;running:true}} }
 HostApi { id:api;pluginId:"io.voxtype.settings"
  _summon:function(id,payload){if(id!==pluginId||!panel.item)return false;panel.item.open(payload);return true}
  _hide:function(id){if(id!==pluginId||!panel.item)return false;panel.item.close();return true}
  _isOpen:function(id){return id===pluginId&&panel.item&&panel.item.opened} }
 Loader { id:panel;asynchronous:true;source:PANEL_URL
  onLoaded:{item.shell=api;item.manifest=fixture.publicManifest} }
 TestCase { id:tc;when:false;name:"PrivateHistorySettings";parent:panel.item?panel.item.renderTest:null }
 function shown(item){for(var p=item;p;p=p.parent)if(!p.visible)return false;return true}
 function find(item,predicate){if(!item)return null;if(predicate(item))return item;var c=item.children||[];for(var i=0;i<c.length;i++){var hit=find(c[i],predicate);if(hit)return hit}return null}
 function row(key){var f=panel.item.formTest;for(var i=0;i<f.visibleKeys.length;i++)if(f.visibleKeys[i].key===key){f.scrollTo(i+f.leadingRows);return f.rowItem(i+f.leadingRows)}return null}
 function control(key){var r=row(key);if(!r||!r.control)throw Error("Missing real row "+key);return r.control}
 function widget(key){var c=control(key);var w=c.controlKind==="bool"?find(c,function(x){return x.checked!==undefined&&shown(x)}):find(c,function(x){return x.objectName===(c.controlKind==="int"?"settingNumberField":"settingTextField")});if(!w||!shown(w)||!w.enabled||w.width<=0||w.height<=0)throw Error("Unrendered control "+key);return w}
 IpcHandler { target:"fixture"
  function status():string {
   var p=panel.item;var f=p?p.formTest:null;var values={};if(f)for(var i=0;i<f.visibleKeys.length;i++)values[f.visibleKeys[i].key]=f.visibleKeys[i].value
   return JSON.stringify({ready:p!==null,opened:p?p.opened:false,section:p?p.activeSection:"",view:p?p.view:"",config:p?p.configPath:"",restartNeeded:p?p.restartNeeded:false,schemaError:p?p.schemaError:"",schemaLoading:p?p.cliTest.schemaLoading:false,bin:p?p.cliTest.bin:"",values:values,error:fixture.lastError,serial:fixture.serial}) }
  function input(key:string,mode:string,value:string):string {
   fixture.lastError="";var serial=++fixture.serial
   Qt.callLater(function(){try{
    var w=fixture.widget(key);tc.wait(100);tc.mouseClick(w,w.width/2,w.height/2)
    if(mode!=="toggle"){
     var editor=w.field!==undefined?w.field:w
     if(!editor.activeFocus)throw Error("Real editor did not receive focus: "+key)
     tc.keyClick(Qt.Key_A,Qt.ControlModifier);tc.keyClick(Qt.Key_Delete)
     for(var i=0;i<value.length;i++)tc.keyClick(value.charAt(i))
     tc.keyClick(mode==="cancel"?Qt.Key_Escape:Qt.Key_Return)
    }
    console.log("ACTUAL_SETTINGS_INPUT",JSON.stringify({serial:serial,key:key,mode:mode,value:value}))
   }catch(e){fixture.lastError=String(e);console.error("ACTUAL_SETTINGS_INPUT_ERROR",String(e))}fixture.completed=serial})
   return String(serial)
  }
  function completion():string{return JSON.stringify({completed:fixture.completed,error:fixture.lastError})}
 }
 property int serial:0
 property int completed:0
 property string lastError:""
}
'''.replace('MANIFEST',json.dumps(manifest)).replace('PANEL_URL',json.dumps(entry.as_uri())))
 (root/'sway.conf').write_text('output HEADLESS-1 mode 1600x1200\nxwayland disable\nseat * fallback true\nfocus_follows_mouse no\n')
 def start(argv,label):
  f=(root/(label+'.log')).open('w');logs.append(f);p=subprocess.Popen(argv,env=env,stdin=subprocess.DEVNULL,stdout=f,stderr=subprocess.STDOUT);processes.append(p);return p
 def ipc(target,method,*args):
  p=subprocess.run(['qs','ipc','-p',str(qml),'call',target,method,*args],env=env,capture_output=True,text=True,timeout=5)
  if p.returncode:return None
  return p.stdout.strip()
 def status():
  value=ipc('fixture','status');return json.loads(value) if value else {}
 def wait(fn,message,seconds=35):
  until=time.monotonic()+seconds
  while True:
   value=fn()
   if value:return value
   assert all(p.poll() is None for p in processes),(message,[(p.args,p.returncode) for p in processes])
   assert time.monotonic()<until,(message,status())
   time.sleep(.05)
 def saved():
  assert sentinel.read_bytes()==untouched,'Wrong config authority'
  assert '# preserve model' in chosen.read_text() and '# preserve unrelated' in chosen.read_text()
  assert chosen.stat().st_mode&0o777==0o600
  return tomllib.loads(chosen.read_text()).get('history',{})
 def resolved():
  p=subprocess.run([str(exe),*(['--config',str(chosen)] if case=='explicit' else []),'config','get','--json'],env=env,capture_output=True,text=True,timeout=10);assert p.returncode==0,p.stderr;return json.loads(p.stdout)
 def change(key,mode,value,expected,restart_expected=False):
  serial=int(ipc('fixture','input',key,mode,value) or '0');assert serial>0
  def done():
   text=ipc('fixture','completion');r=json.loads(text) if text else {};assert not r.get('error'),r
   return r.get('completed')==serial
  wait(done,'Actual native pointer/key event did not complete')
  wait(lambda:resolved()[key]==expected,'Canonical resolved value not persisted for '+key)
  wait(lambda:status().get('values',{}).get(key)==expected,'Actual rendered schema value not reloaded for '+key)
  assert status()['restartNeeded']==restart_expected,'Live-reloaded History controls must not require a restart or erase an unrelated pending restart'
  saved();steps.append(dict(key=key,mode=mode,value=value,canonical=expected,serial=serial))
 def shot(label):subprocess.run(['grim','-o','HEADLESS-1','/work/out/history-settings-'+case+'-'+label+'.png'],env=env,check=True,timeout=5)
 try:
  start(['sway','-c',str(root/'sway.conf')],'sway')
  def sockets():
   w=[p for p in runtime.glob('wayland-*') if stat.S_ISSOCK(p.lstat().st_mode)];i=list(runtime.glob('sway-ipc*.sock'))
   if w and i:env.update(WAYLAND_DISPLAY=w[0].name,SWAYSOCK=str(i[0]));return True
  wait(sockets,'Private Sway sockets')
  start(['qs','-n','-p',str(qml)],'panel');wait(lambda:status().get('ready'),'Staged canonical Panel loader')
  assert ipc('io.voxtype.settings','section','History')=='ok'
  wait(lambda:status().get('section')=='History' and len(status().get('values',{}))==6,'Rendered actual History schema rows')
  assert status()['config']==str(chosen) and status()['view']=='settings'
  assert 'history' not in tomllib.loads(chosen.read_text()),'Opening old settings must not opt in'
  initial=resolved();assert not initial['history.enabled'] and not initial['history.paused'] and initial['history.record_app'] and not initial['history.record_window_title'] and initial['history.retention_days']==30
  shot('initial')
  change('history.enabled','toggle','',True)
  change('history.paused','toggle','',True)
  generation_path=root/'data/voxtype/history-control'/hashlib.sha256(os.fsencode(chosen.resolve())).hexdigest()/'generation'
  assert int.from_bytes(generation_path.read_bytes(),'little')==1
  change('history.exclude_apps','save','private-editor-q, Bitwarden','private-editor-q,Bitwarden')
  change('history.retention_days','save','0',0)
  change('history.record_app','toggle','',False)
  change('history.record_window_title','toggle','',True)
  shot('saved')
  before_cancel=chosen.read_bytes()
  change('history.exclude_apps','cancel','private-cancelled-change','private-editor-q,Bitwarden')
  assert chosen.read_bytes()==before_cancel,'Escape must not save a cancelled privacy edit'
  change('history.exclude_apps','save','','')
  assert saved()['exclude_apps']==''
  assert ipc('io.voxtype.settings','close')=='ok'
  wait(lambda:not status().get('opened'),'Actual panel close')
  assert ipc('io.voxtype.settings','section','History')=='ok'
  wait(lambda:status().get('opened') and status().get('values',{}).get('history.exclude_apps')=='','Actual reopen reload')
  assert resolved()['history.paused'] and resolved()['history.retention_days']==0
  shot('reopened')
  change('history.paused','toggle','',False)
  change('history.enabled','toggle','',False)
  assert int.from_bytes(generation_path.read_bytes(),'little')==2
  assert generation_path.stat().st_mode&0o777==0o600
  # Regress the sibling contract through real existing controls: non-History
  # writes still require restart, and later History writes cannot clear it.
  assert ipc('io.voxtype.settings','section','Text Cleanup')=='ok'
  wait(lambda:status().get('section')=='Text Cleanup' and 'text.filter_filler_words' in status().get('values',{}),'Existing non-History restart control')
  change('text.filter_filler_words','toggle','',True,True)
  assert ipc('io.voxtype.settings','section','History')=='ok'
  wait(lambda:status().get('section')=='History' and len(status().get('values',{}))==6,'Return to actual History form')
  change('history.record_app','toggle','',True,True)
  change('history.record_app','toggle','',False,True)
  assert ipc('io.voxtype.settings','section','Text Cleanup')=='ok'
  wait(lambda:status().get('section')=='Text Cleanup' and 'text.filter_filler_words' in status().get('values',{}),'Restore controlled sibling value')
  change('text.filter_filler_words','toggle','',False,True)
  assert tomllib.loads(chosen.read_text())['text']['filter_filler_words'] is False
  assert int.from_bytes(generation_path.read_bytes(),'little')==2
  assert ipc('io.voxtype.settings','section','History')=='ok'
  wait(lambda:status().get('section')=='History' and len(status().get('values',{}))==6,'Final History capture')
  assert not select.select([audio],[],[],0)[0],'No audio probe may reach the private sentinel'
  assert hashlib.sha256(exe.read_bytes()).hexdigest()==native_sha
  passed=True
 except BaseException as error:
  failure=repr(error);raise
 finally:
  current_status=status() if len(processes)>1 and processes[-1].poll() is None else {}
  if processes and processes[0].poll() is None and env.get('WAYLAND_DISPLAY'):
   try:shot('final')
   except BaseException as e:steps.append(dict(capture_error=repr(e)))
  for p in reversed(processes):
   if p.poll() is None:p.terminate()
   try:p.wait(timeout=5)
   except subprocess.TimeoutExpired:p.kill();p.wait()
  for f in logs:f.close()
  for label in ['panel','sway']:
   p=root/(label+'.log')
   if p.exists():Path('/work/out/history-settings-'+case+'-'+label+'.log').write_bytes(p.read_bytes())
  native_records=[json.loads(line) for line in journal.read_text().splitlines()] if journal.exists() else []
  audio_attempted=bool(select.select([audio],[],[],0)[0]);audio.close()
  result=dict(passed=passed,case=case,failure=failure,steps=steps,status=current_status,native_sha256=native_sha,native_records=native_records,unselected_sentinel_unchanged=sentinel.read_bytes()==untouched,audio_attempted=audio_attempted,private_config_mode=oct(chosen.stat().st_mode&0o777),umask_during_graphical_saves='022',instrumentation='Real staged plugin plus read-only form/cli/card aliases, real QtTest pointer/key events on actual layer-shell controls; transparent real native subprocess observer prepends chosen config only in explicit case, refuses unrelated info-devices with77; reduced real Omarchy HostApi/async Loader, not installed/full desktop.')
  Path('/work/out/history-settings-'+case+'.json').write_text(json.dumps(result,indent=2)+'\n');print('ACTUAL_GRAPHICAL_HISTORY_SETTINGS',json.dumps(result),flush=True)
