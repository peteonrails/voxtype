"""Actual unique Meetings window: two private stores through canonical forwarding."""
import datetime,hashlib,json,os,select,signal,sqlite3,subprocess,tempfile,time
from pathlib import Path
import pyatspi
from gi.repository import Gio,GLib
assert os.environ.get('HOME')=='/work/home' and os.environ.get('DISPLAY') and os.environ.get('XAUTHORITY') and os.environ.get('DBUS_SESSION_BUS_ADDRESS')
assert not os.environ.get('WAYLAND_DISPLAY')
os.umask(0o077)
exe=Path(os.environ.get('VOXTYPE_HISTORY_TEST_BIN','/work/target/debug/voxtype'))
helper=Path(os.environ.get('VOXTYPE_HISTORY_TEST_GTK_BIN','/work/target/debug/voxtype-meetings-gtk4'))
case=os.environ.get('VOX_MEETING_FORWARD_CASE','absolute')
assert case in ['absolute','relative','action','default','missing','early','stale','direct-relative']
bus=Gio.bus_get_sync(Gio.BusType.SESSION,None)
def walk(node,depth=0):
 if depth>25:return
 try:count=node.childCount;node.name
 except GLib.Error:return
 yield node
 for i in range(count):
  try:child=node.getChildAtIndex(i)
  except GLib.Error:continue
  if child is not None:yield from walk(child,depth+1)
def showing():
 result=[]
 for node in walk(pyatspi.Registry.getDesktop(0)):
  try:
   if node.getState().contains(pyatspi.STATE_SHOWING):result.append({'name':node.name,'role':node.getRoleName()})
  except GLib.Error:pass
 return result
def visible(text):return any(text in x['name'] for x in showing())
def wait(text):
 deadline=time.monotonic()+8
 while not visible(text):
  assert time.monotonic()<deadline,('Actual already-running Meetings window must show selected store-specific saved meeting',text,showing())
  time.sleep(.05)
def owner():return bus.call_sync('org.freedesktop.DBus','/org/freedesktop/DBus','org.freedesktop.DBus','GetConnectionUnixProcessID',GLib.Variant('(s)',('io.voxtype.Meetings',)),GLib.VariantType.new('(u)'),Gio.DBusCallFlags.NONE,1000,None).unpack()[0]
with tempfile.TemporaryDirectory(prefix='meeting-store-forward-',dir='/work/scratch') as name:
 root=Path(name);runtime=root/'runtime';runtime.mkdir(mode=0o700)
 env=dict(os.environ,HOME=str(root),XDG_CONFIG_HOME=str(root/'config'),XDG_DATA_HOME=str(root/'data'),XDG_RUNTIME_DIR=str(runtime),GDK_BACKEND='x11',GSK_RENDERER='cairo',GTK_A11Y='atspi')
 assert not env.get('VOXTYPE_MEETINGS_GTK4_SMOKE'),'Exercise normal unique application, not smoke mode'
 now=int(datetime.datetime.now().timestamp());meeting='11111111-2222-4333-8444-555555555555';configs={}
 for label in ['A','B','D']:
  directory=root/label;directory.mkdir();store=directory/'meetings';config=directory/'config.toml'
  config.write_text('[hotkey]\nenabled=false\n[meeting]\nview="gtk4"\nstorage_path='+json.dumps(str(store))+'\n')
  config.write_text(config.read_text()+'[meeting.assistant]\nagent="private-unsupported-'+label+'"\n')
  bootstrap=subprocess.run([str(exe),'--config',str(config),'meeting','list','--json'],env=env,capture_output=True,text=True,timeout=10)
  assert bootstrap.returncode==0,bootstrap.stderr
  saved=store/'synthetic';saved.mkdir()
  transcript={'segments':[{'id':1,'start_ms':0,'end_ms':1000,'text':'Private transcript '+label+' only.','source':'unknown','chunk_id':0}],'total_chunks':1}
  (saved/'transcript.json').write_text(json.dumps(transcript))
  with sqlite3.connect(store/'index.db') as db:db.execute('INSERT INTO meetings(id,title,started_at,ended_at,duration_secs,status,chunk_count,storage_path,audio_retained) VALUES(?,?,?,?,?,?,?,?,?)',(meeting,'Private meeting '+label+' only',now,now,1,'completed',1,str(saved),0))
  direct=subprocess.run([str(exe),'--config',str(config),'meeting','show',meeting,'--json'],env=env,capture_output=True,text=True,timeout=10)
  assert direct.returncode==0 and json.loads(direct.stdout)['metadata']['title']=='Private meeting '+label+' only'
  configs[label]=config
 default_config=root/'config/voxtype/config.toml';default_config.parent.mkdir(parents=True)
 default_config.write_bytes(configs['D'].read_bytes())
 primary=None;records=[];held=None;obsolete_pid=None;direct_primary=None
 try:
  sequence=['A','D','B','D','A'] if case=='default' else ['A','B'] if case=='early' else ['A','B','A']
  for position,label in enumerate(sequence):
   path=None if label=='D' else 'config.toml' if case in ['relative','direct-relative'] else str(configs[label])
   command=([str(helper),'--config',path,'--meeting',meeting] if case=='direct-relative' else [str(exe),*(['--config',path] if path is not None else []),'meeting','view',meeting])
   if case=='stale':command+=['--tab','ask']
   if case=='direct-relative' and position==0:
    direct_primary=subprocess.Popen(command,cwd=configs[label].parent,env=env,stdin=subprocess.DEVNULL,stdout=subprocess.DEVNULL,stderr=subprocess.PIPE,text=True)
    print('ACTUAL_DIRECT_PRIMARY_STARTED',direct_primary.pid,command,flush=True)
   else:
    p=subprocess.run(command,cwd=configs[label].parent,env=env,capture_output=True,text=True,timeout=10)
    print('ACTUAL_CANONICAL_MEETING_FORWARD',label,case,p.returncode,p.stdout,p.stderr,flush=True)
    assert p.returncode==0,p.stderr
   if primary is None:
    deadline=time.monotonic()+8
    while primary is None:
     try:primary=owner()
     except GLib.Error:
      assert time.monotonic()<deadline,'Initial real GTK window must acquire D-Bus ownership'
      time.sleep(.05)
   if not (case=='early' and position==0):
    wait('Private meeting '+label+' only');wait('Private transcript '+label+' only.')
   if case in ['absolute','relative','action','default','missing','stale','direct-relative']:
    # Exercise a real GUI command without any agent, model or network access.
    reference=subprocess.run([str(exe),'--config',str(configs[label]),'meeting','ask',meeting,'Private routing probe','--json'],env=env,capture_output=True,text=True,timeout=10)
    assert 'private-unsupported-'+label in reference.stdout,(reference.returncode,reference.stdout,reference.stderr)
    # This public quick-question route calls the same GUI start_ask callback
    # as its entry, without depending on unimplemented AT-SPI focus methods.
    question='What are the action items so far?'
    requested=subprocess.run(command+['--ask',question if case=='direct-relative' else 'actions'],cwd=configs[label].parent,env=env,capture_output=True,text=True,timeout=10)
    assert requested.returncode==0,requested.stderr
    wait('What are the action items so far?')
    wait('private-unsupported-'+label)
    print('REAL_GUI_CANONICAL_ACTION_TARGET',label,reference.stdout,flush=True)
   if case=='stale' and position==0:
    # A real CLI read waits behind this private SQLite transaction while B
    # remains usable. No substitute CLI, provider or IPC endpoint is involved.
    held=sqlite3.connect(configs['A'].parent/'meetings/index.db')
    held.execute('BEGIN EXCLUSIVE')
    entries=[n for n in walk(pyatspi.Registry.getDesktop(0)) if n.getRoleName() in ['text','entry'] and n.getState().contains(pyatspi.STATE_SHOWING) and n.getState().contains(pyatspi.STATE_EDITABLE)]
    assert len(entries)==1
    entries[0].queryEditableText().setTextContents('Private delayed A probe')
    window=subprocess.check_output(['xdotool','search','--onlyvisible','--pid',str(primary),'--name','^Meetings - Voxtype$'],env=env,text=True).splitlines();assert len(window)==1
    subprocess.run(['xdotool','windowfocus',window[0]],env=env,check=True,timeout=5)
    # --tab ask used the product's own grab_focus, not AT-SPI's unsupported one.
    subprocess.run(['xdotool','key','--clearmodifiers','Return'],env=env,check=True,timeout=5)
    deadline=time.monotonic()+3
    while obsolete_pid is None:
     for d in Path('/proc').iterdir():
      if not d.name.isdigit():continue
      try:
       argv=d.joinpath('cmdline').read_bytes().split(b'\0')
       if argv and argv[0]==os.fsencode(exe) and b'ask' in argv and os.fsencode(configs['A']) in argv:
        if any(os.path.realpath(f)==str(configs['A'].parent/'meetings/index.db') for f in d.joinpath('fd').iterdir()):obsolete_pid=int(d.name);break
      except (FileNotFoundError,ProcessLookupError):pass
     assert time.monotonic()<deadline,'Actual canonical A ask must reach its held database before B forwarding'
     if obsolete_pid is None:time.sleep(.02)
    print('ACTUAL_BLOCKED_A_CLI',obsolete_pid,flush=True)
   if case=='stale' and position==1:
    assert held is not None and obsolete_pid is not None
    descriptor=os.pidfd_open(obsolete_pid)
    held.rollback();held.close();held=None
    assert select.select([descriptor],[],[],5)[0],'Old actual A CLI must exit after barrier release'
    os.close(descriptor)
    deadline=time.monotonic()+1
    while time.monotonic()<deadline:
     assert visible('private-unsupported-B') and not visible('private-unsupported-A'),'Released A replies must not replace current B notices'
     time.sleep(.05)
    print('ACTUAL_OBSOLETE_A_RELEASED_B_RETAINED',obsolete_pid,flush=True)
   assert owner()==primary,'Intended normal Meetings frontend remains a single window/process'
   names=showing()
   for other in ['A','B','D']:
    if other!=label and not(case=='early' and position==0):assert not any('Private transcript '+other+' only.' in n['name'] for n in names)
   records.append({'selected_store':label,'same_meeting_id':meeting,'config_argument':path,'caller_cwd':str(configs[label].parent),'actual_owner':primary,'showing':names})
   if case in ['absolute','relative','default','direct-relative']:
    back=next(n for n in walk(pyatspi.Registry.getDesktop(0)) if n.getRoleName() in ['button','push button'] and n.name=='Meetings' and n.getState().contains(pyatspi.STATE_SHOWING))
    assert back.queryAction().doAction(0),'Real Meetings back button must execute its storage-backed list action'
    wait('Recent meetings');wait('Private meeting '+label+' only')
    names=showing()
    assert not any('Private transcript ' in n['name'] for n in names),'Back action must actually show the list, not old transcript'
    print('REAL_GUI_LIST_STORE_TARGET',label,json.dumps(names),flush=True)
   if case=='missing' and position==2:
    missing=root/'absent-parent/config.toml'
    result=subprocess.run([str(helper),'--meeting',meeting,'--config',str(missing)],env=env,capture_output=True,text=True,timeout=10)
    assert result.returncode!=0 and str(missing) in result.stderr,(result.returncode,result.stdout,result.stderr)
    assert not missing.parent.exists(),'Missing explicit config must never create or overwrite anything'
    assert visible('Private transcript A only.') and owner()==primary,'Invalid forwarded config must preserve previous valid context'
    print('ACTUAL_FORWARDED_CONFIG_ERROR',result.returncode,result.stderr,flush=True)
  if case=='early':
   time.sleep(.5)
   wait('Private transcript B only.')
   assert not visible('Private transcript A only.'),'Initial delayed A load must not replace forwarded B'
  from PIL import ImageGrab
  ImageGrab.grab(xdisplay=env['DISPLAY']).save('/work/out/meeting-store-forward-'+case+'.png')
  Path('/work/out/meeting-store-forward-'+case+'.json').write_text(json.dumps(records,indent=2))
  print(json.dumps({'case':case,'actual_single_window_store_sequence':sequence,'owner':primary,'passed':True}),flush=True)
 finally:
  if held is not None:held.rollback();held.close()
  if primary is not None:
   fd=os.pidfd_open(primary)
   try:signal.pidfd_send_signal(fd,signal.SIGTERM)
   finally:os.close(fd)
  if direct_primary is not None:
   direct_primary.wait(timeout=5)
   print('ACTUAL_DIRECT_PRIMARY_FIXTURE_TERMINATED',direct_primary.returncode,direct_primary.stderr.read(),flush=True)
