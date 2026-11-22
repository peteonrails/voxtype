"""Normal unique GTK application: actual forwarded --config and mutations.
No NON_UNIQUE smoke flag; private Xvfb/D-Bus and synthetic files only.
"""
import datetime,json,os,subprocess,tempfile,time
from pathlib import Path
import pyatspi
from gi.repository import Gio,GLib
assert os.environ.get('HOME')=='/work/home'
assert os.environ.get('DISPLAY') and os.environ.get('DBUS_SESSION_BUS_ADDRESS')
assert not any(os.environ.get(key) for key in ['WAYLAND_DISPLAY','SWAYSOCK','NIRI_SOCKET','HYPRLAND_INSTANCE_SIGNATURE','VOXTYPE_MEETINGS_GTK4_SMOKE'])
os.umask(0o077)
cli=os.environ.get('VOXTYPE_HISTORY_TEST_BIN','/work/target/debug/voxtype')
gtk=os.environ.get('VOXTYPE_HISTORY_TEST_GTK_BIN','/work/target/debug/voxtype-meetings-gtk4')
case=os.environ.get('VOX_HISTORY_GTK_FORWARD_PATH','absolute')
assert case in ['absolute','same-relative','different-relative','direct-same-relative','direct-different-relative','default-relative','missing-parent-relative']
with tempfile.TemporaryDirectory(dir='/work/scratch',prefix='history-gtk-forwarding-') as name:
 root=Path(name); runtime=root/'runtime'; runtime.mkdir(mode=0o700)
 folder=root/'data/voxtype/history'; folder.mkdir(parents=True,mode=0o700)
 now=datetime.datetime.now().astimezone(); entry_id=now.strftime('%Y%m%d-%H%M%S')+'-abc123'
 (folder/(now.strftime('%Y-%m-%d')+'.jsonl')).write_text(json.dumps(dict(schema=1,id=entry_id,ts_start=now.isoformat(),ts_end=now.isoformat(),raw_text='synthetic normal GTK',text='Synthetic normal GTK.',engine='whisper',model='synthetic.gguf',delivered=True,outcome='delivered'))+'\n')
 cwd_a=root/'caller-a'; cwd_b=root/'caller-b'
 cwd_a.mkdir(); cwd_b.mkdir()
 a=cwd_a/'config.toml'; b=cwd_b/'config.toml'
 if case=='default-relative':
  a=root/'config/voxtype/config.toml'; a.parent.mkdir(parents=True)
 a.write_text('[history]\nenabled=true\npaused=false\n')
 if case=='missing-parent-relative':
  blocker=cwd_b/'blocker'; blocker.write_text('Synthetic non-directory config parent')
  b=blocker/'config.toml'
 else: b.write_text('[history]\nenabled=false\npaused=false\n')
 a_args=[] if case=='default-relative' else ['--config',str(a) if case=='absolute' else 'config.toml']
 b_argument=str(b) if case=='absolute' else ('./config.toml' if 'different-relative' in case else 'blocker/config.toml' if case=='missing-parent-relative' else 'config.toml')
 env=dict(os.environ,HOME=str(root),XDG_DATA_HOME=str(root/'data'),XDG_CONFIG_HOME=str(root/'config'),XDG_RUNTIME_DIR=str(runtime),GDK_BACKEND='x11',GSK_RENDERER='cairo',GTK_A11Y='atspi')
 def walk(node,depth=0):
  if depth>25: return
  try: count=node.childCount; node.name
  except GLib.Error: return
  yield node
  for index in range(count):
   try: child=node.getChildAtIndex(index)
   except GLib.Error: continue
   if child is not None: yield from walk(child,depth+1)
 def nodes(): return list(walk(pyatspi.Registry.getDesktop(0)))
 def showing(node):
  try: return node.getState().contains(pyatspi.STATE_SHOWING)
  except GLib.Error: return False
 def frame_with(text):
  for frame in nodes():
   if frame.getRoleName() in ['frame','window'] and showing(frame):
    if any(n.name==text and showing(n) for n in walk(frame)): return frame
  return None
 def wait(check,message,timeout=8):
  deadline=time.monotonic()+timeout
  while True:
   while GLib.MainContext.default().pending(): GLib.MainContext.default().iteration(False)
   value=check()
   if value: return value
   assert gui.poll() is None, 'Normal GTK primary exited'
   assert time.monotonic()<deadline,message
   time.sleep(0.05)
 def activate(frame,label):
  button=wait(lambda:next((n for n in walk(frame) if n.name==label and showing(n) and n.getRoleName() in ['push button','button']),None),'Missing control '+label)
  assert button.queryAction().doAction(0)
 def state(path):
  result=subprocess.run([cli,'--config',str(path),'history','--json','status'],env=env,capture_output=True,text=True,timeout=10)
  assert result.returncode==0,result.stderr
  return json.loads(result.stdout)
 connection=Gio.bus_get_sync(Gio.BusType.SESSION,None)
 def primary_pid():
  reply=connection.call_sync('org.freedesktop.DBus','/org/freedesktop/DBus','org.freedesktop.DBus','GetConnectionUnixProcessID',GLib.Variant('(s)',('io.voxtype.History',)),GLib.VariantType.new('(u)'),Gio.DBusCallFlags.NONE,1000,None)
  return reply.unpack()[0]
 with (root/'primary.stdout').open('xb') as stdout,(root/'primary.stderr').open('xb') as stderr:
  gui=subprocess.Popen([gtk,'--history',*a_args],cwd=cwd_a,env=env,stdin=subprocess.DEVNULL,stdout=stdout,stderr=stderr)
  try:
   wait(lambda:frame_with('History on'),'Initial unique GTK window did not load config A')
   assert primary_pid()==gui.pid, 'Fixture must exercise the real unique application owner'
   invocation=[gtk,'--history','--config',b_argument] if case.startswith('direct-') else [cli,'--config',b_argument,'history','view']
   launch=subprocess.run(invocation,cwd=cwd_b,env=env,capture_output=True,text=True,timeout=12)
   assert launch.returncode==0,launch.stdout+launch.stderr
   frame=wait(lambda:frame_with('History off'),'Forwarded --config B must show B collection state, not silently keep A',timeout=5)
   assert primary_pid()==gui.pid, 'Forwarding must reach the same unique GTK primary'
   activate(frame,'Turn on')
   if case=='missing-parent-relative':
    expected=subprocess.run([cli,'--config',str(b),'history','--json','enable'],env=env,capture_output=True,text=True,timeout=10)
    assert expected.returncode!=0, 'Synthetic non-directory config parent must retain its normal write error'
    error=json.loads(expected.stdout)['error']
    wait(lambda:any(n.name=='History change did not complete: '+error and showing(n) for n in walk(frame)),'Forwarded nonexistent config must expose its own write error, not target A')
    assert not b.exists()
   else:
    wait(lambda:state(b)['enabled'],'Enable in B window must persist B')
    wait(lambda:any(n.name=='History on' and showing(n) for n in walk(frame)),'B window did not refresh')
    activate(frame,'Pause')
    wait(lambda:state(b)['paused'],'Pause in B window must persist B')
   assert state(a)['enabled'] is True and state(a)['paused'] is False, 'Forwarded B controls must never mutate configuration A'
   print(json.dumps({'case':'normal-unique-gtk-forwarded-config','path_case':case,'argument':b_argument,'caller_a':str(cwd_a),'caller_b':str(cwd_b),'primary_pid':gui.pid,'a':state(a),'b':state(b),'scope':'actual HANDLES_COMMAND_LINE forwarding through public CLI or supported helper and scoped GTK actions; no NON_UNIQUE smoke mode or production session'}))
  finally:
   if gui.poll() is None:
    gui.terminate()
    try: gui.wait(timeout=5)
    except subprocess.TimeoutExpired: gui.kill(); gui.wait(timeout=5)
   print((root/'primary.stdout').read_text(errors='replace'))
   print((root/'primary.stderr').read_text(errors='replace'))
