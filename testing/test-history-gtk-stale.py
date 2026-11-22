"""Real unique GTK: an obsolete SQLite-blocked search must not replace clear.
All history/config/runtime and both application windows are private NAS fixtures.
"""
import datetime,json,os,sqlite3,subprocess,tempfile,time
from pathlib import Path
import pyatspi
from gi.repository import GLib
assert os.environ.get('HOME')=='/work/home'
assert os.environ.get('DISPLAY') and os.environ.get('DBUS_SESSION_BUS_ADDRESS')
assert not any(os.environ.get(k) for k in ['WAYLAND_DISPLAY','SWAYSOCK','NIRI_SOCKET','HYPRLAND_INSTANCE_SIGNATURE','VOXTYPE_MEETINGS_GTK4_SMOKE'])
os.umask(0o077)
cli=os.environ.get('VOXTYPE_HISTORY_TEST_BIN','/work/target/debug/voxtype')
gtk=os.environ.get('VOXTYPE_HISTORY_TEST_GTK_BIN','/work/target/debug/voxtype-meetings-gtk4')
case=os.environ.get('VOX_HISTORY_GTK_STALE','success')
assert case in ['success','error']
with tempfile.TemporaryDirectory(dir='/work/scratch',prefix='history-gtk-stale-') as name:
 root=Path(name); runtime=root/'runtime'; runtime.mkdir(mode=0o700)
 folder=root/'data/voxtype/history'; folder.mkdir(parents=True,mode=0o700)
 now=datetime.datetime.now().astimezone()
 def entry(suffix,text,delta):
  stamp=now+datetime.timedelta(microseconds=delta)
  return dict(schema=1,id=stamp.strftime('%Y%m%d-%H%M%S')+'-'+suffix,ts_start=stamp.isoformat(),ts_end=stamp.isoformat(),raw_text=text,text=text,engine='whisper',model='synthetic.gguf',delivered=True,outcome='delivered')
 items=[entry('abc123','Needleold original.',0),entry('abc124','Other initial.',1)]
 day=folder/(now.strftime('%Y-%m-%d')+'.jsonl')
 day.write_text(''.join(json.dumps(item)+'\n' for item in items))
 a=root/'a.toml'; c=root/'c.toml'
 a.write_text('[history]\nenabled=true\n'); c.write_text('[history]\nenabled=false\n')
 env=dict(os.environ,HOME=str(root),XDG_DATA_HOME=str(root/'data'),XDG_CONFIG_HOME=str(root/'config'),XDG_RUNTIME_DIR=str(runtime),XDG_SESSION_TYPE='x11',GDK_BACKEND='x11',GSK_RENDERER='cairo',GTK_A11Y='atspi')
 indexed=subprocess.run([cli,'--config',str(a),'history','--json','reindex'],env=env,capture_output=True,text=True,timeout=10)
 assert indexed.returncode==0,indexed.stderr
 def walk(node,depth=0):
  if depth>25:return
  try: count=node.childCount; node.name
  except GLib.Error:return
  yield node
  for i in range(count):
   try: child=node.getChildAtIndex(i)
   except GLib.Error:continue
   if child is not None:yield from walk(child,depth+1)
 def showing(node):
  try:return node.getState().contains(pyatspi.STATE_SHOWING)
  except GLib.Error:return False
 def nodes():return list(walk(pyatspi.Registry.getDesktop(0)))
 def frame_with(text):
  return next((n for n in nodes() if n.getRoleName() in ['frame','window'] and showing(n) and any(x.name==text and showing(x) for x in walk(n))),None)
 def wait(check,message,timeout=12):
  deadline=time.monotonic()+timeout
  while True:
   while GLib.MainContext.default().pending():GLib.MainContext.default().iteration(False)
   result=check()
   if result:return result
   assert gui.poll() is None,'Normal unique GTK primary exited'
   assert time.monotonic()<deadline,message
   time.sleep(0.02)
 def command_pid():
  pending=[gui.pid]
  for _ in range(100):
   if not pending:return None
   pid=pending.pop()
   try:
    args=Path('/proc/'+str(pid)+'/cmdline').read_bytes().split(b'\0')
    if b'history' in args and b'search' in args and b'needleold' in args:return pid
    for task in Path('/proc/'+str(pid)+'/task').iterdir():
     try:pending.extend(int(v) for v in (task/'children').read_text().split())
     except FileNotFoundError:pass
   except FileNotFoundError:pass
  raise AssertionError('Unexpectedly large owned process tree')
 def workers_finished():
  for task in Path('/proc/'+str(gui.pid)+'/task').iterdir():
   try:
    if (task/'comm').read_text().startswith('voxtype-history'):return False
   except FileNotFoundError:pass
  return True
 held=None
 with (root/'gui.stdout').open('xb') as out,(root/'gui.stderr').open('xb') as err:
  gui=subprocess.Popen([gtk,'--history','--config',str(a)],env=env,stdin=subprocess.DEVNULL,stdout=out,stderr=err)
  try:
   frame=wait(lambda:frame_with('History on'),'Primary did not load')
   wait(lambda:any(n.name=='Other initial.' and showing(n) for n in walk(frame)),'Initial list did not render')
   search=wait(lambda:next((n for n in walk(frame) if n.getRoleName() in ['text','entry'] and showing(n) and n.getState().contains(pyatspi.STATE_EDITABLE)),None),'Search field missing')
   held=sqlite3.connect(root/'data/voxtype/meetings/index.db'); held.execute('BEGIN IMMEDIATE')
   assert search.queryEditableText().setTextContents('needleold')
   pid=wait(command_pid,'Actual obsolete search did not start')
   db=str(root/'data/voxtype/meetings/index.db')
   wait(lambda:any(os.path.realpath(fd)==db for fd in Path('/proc/'+str(pid)+'/fd').iterdir()),'Search did not reach actual locked SQLite database')
   newest=entry('abc125','Newest clear result.',2)
   with day.open('a') as stream:stream.write(json.dumps(newest)+'\n')
   assert search.queryEditableText().setTextContents('')
   wait(lambda:any(n.name==newest['text'] and showing(n) for n in walk(frame)),'New clear/list must render before releasing obsolete search')
   assert command_pid()==pid,'Obsolete native search must still be pending at clear completion'
   if case=='success':held.rollback()
   # In error mode the real SQLite busy timeout supplies the obsolete error.
   wait(lambda:command_pid() is None,'Obsolete actual search did not finish',timeout=20)
   wait(workers_finished,'History worker did not finish delivering its native result')
   # A subsequent real unique-app launch and native list completion constitute
   # a GLib-loop barrier after every earlier history worker has sent its reply.
   forwarded=subprocess.run([gtk,'--history','--config',str(c)],env=env,capture_output=True,text=True,timeout=5)
   assert forwarded.returncode==0,forwarded.stderr
   other=wait(lambda:frame_with('History off'),'Main loop failed to handle subsequent unique invocation')
   wait(lambda:any(n.name==newest['text'] and showing(n) for n in walk(other)),'Subsequent native list did not complete')
   current=[n.name for n in walk(frame) if showing(n)]
   assert newest['text'] in current and 'Other initial.' in current,'Obsolete search replaced the current cleared list'
   assert not any(s.startswith('History search did not complete:') for s in current),'Obsolete search error replaced current notice'
   assert search.queryText().getText(0,-1)=='','Fixture must leave the actual current query empty'
   print(json.dumps({'case':'obsolete-'+case,'actual_search_pid':pid,'clear_rendered_before_release':True,'newest_current':True,'obsolete_notice_visible':False,'native_glib_barrier':True,'scope':'actual CLI SQLite query and normal unique GTK in private Xvfb/DBus; no response stubs or production state'}))
  finally:
   if held is not None:held.rollback();held.close()
   if gui.poll() is None:
    gui.terminate()
    try:gui.wait(timeout=5)
    except subprocess.TimeoutExpired:gui.kill();gui.wait(timeout=5)
   print((root/'gui.stdout').read_text(errors='replace'));print((root/'gui.stderr').read_text(errors='replace'))
