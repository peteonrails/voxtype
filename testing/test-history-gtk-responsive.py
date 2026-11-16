"""Normal GTK must remain responsive while replay waits in a private hook.
The second invocation uses real unique GApplication forwarding, not smoke mode.
"""
import datetime,fcntl,json,os,shlex,sqlite3,subprocess,sys,tempfile,time
from pathlib import Path
import pyatspi
from gi.repository import Gio,GLib
assert os.environ.get('HOME')=='/work/home'
assert os.environ.get('DISPLAY') and os.environ.get('DBUS_SESSION_BUS_ADDRESS')
assert not any(os.environ.get(key) for key in ['WAYLAND_DISPLAY','SWAYSOCK','NIRI_SOCKET','HYPRLAND_INSTANCE_SIGNATURE','VOXTYPE_MEETINGS_GTK4_SMOKE'])
os.umask(0o077)
cli=os.environ.get('VOXTYPE_HISTORY_TEST_BIN','/work/target/debug/voxtype')
gtk=os.environ.get('VOXTYPE_HISTORY_TEST_GTK_BIN','/work/target/debug/voxtype-meetings-gtk4')
case=os.environ.get('VOX_HISTORY_GTK_RESPONSIVENESS','replay')
assert case in ['replay','search','delete']
with tempfile.TemporaryDirectory(dir='/work/scratch',prefix='history-gtk-slow-replay-') as name:
 root=Path(name); runtime=root/'runtime'; runtime.mkdir(mode=0o700)
 folder=root/'data/voxtype/history'; folder.mkdir(parents=True,mode=0o700)
 now=datetime.datetime.now().astimezone(); entry_id=now.strftime('%Y%m%d-%H%M%S')+'-abc123'
 (folder/(now.strftime('%Y-%m-%d')+'.jsonl')).write_text(json.dumps(dict(schema=1,id=entry_id,ts_start=now.isoformat(),ts_end=now.isoformat(),raw_text='synthetic slow replay',text='Synthetic slow replay.',engine='whisper',model='synthetic.gguf',delivered=True,outcome='delivered'))+'\n')
 started=root/'hook-started'; release=root/'release-hook'
 hook=root/'wait-hook.py'
 hook.write_text('import time\nfrom pathlib import Path\nroot=Path(__file__).parent\n(root/"hook-started").write_text("started")\ndeadline=time.monotonic()+20\nwhile not (root/"release-hook").exists():\n if time.monotonic()>deadline: raise SystemExit("private hook release timed out")\n time.sleep(0.02)\n')
 a=root/'configuration-a.toml'; b=root/'configuration-b.toml'
 output_a=root/'a-output.txt'; output_b=root/'b-output.txt'
 a.write_text('[history]\nenabled=true\n[output]\nmode="file"\nfile_path='+json.dumps(str(output_a))+'\npre_output_command='+json.dumps(shlex.join([sys.executable,str(hook)]))+'\nfallback_to_clipboard=false\n')
 b.write_text('[history]\nenabled=false\n[output]\nmode="file"\nfile_path='+json.dumps(str(output_b))+'\nfallback_to_clipboard=false\n')
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
 def wait(check,message,timeout=8):
  deadline=time.monotonic()+timeout
  while True:
   while GLib.MainContext.default().pending(): GLib.MainContext.default().iteration(False)
   value=check()
   if value: return value
   assert gui.poll() is None, 'Normal GTK primary exited'
   assert time.monotonic()<deadline,message
   time.sleep(0.05)
 def activate(label):
  button=wait(lambda:next((n for n in nodes() if n.name==label and showing(n) and n.getRoleName() in ['push button','button']),None),'Missing control '+label)
  assert button.queryAction().doAction(0)
 connection=Gio.bus_get_sync(Gio.BusType.SESSION,None)
 def primary_pid():
  reply=connection.call_sync('org.freedesktop.DBus','/org/freedesktop/DBus','org.freedesktop.DBus','GetConnectionUnixProcessID',GLib.Variant('(s)',('io.voxtype.History',)),GLib.VariantType.new('(u)'),Gio.DBusCallFlags.NONE,1000,None)
  return reply.unpack()[0]
 def blocked_command(action):
  pending=[gui.pid]
  for _ in range(20):
   if not pending: return False
   pid=pending.pop()
   try:
    args=Path('/proc/'+str(pid)+'/cmdline').read_bytes().split(b'\0')
    if b'history' in args and action.encode() in args: return True
    # Linux assigns a child to the spawning thread, not necessarily the GTK
    # main thread. Observe the actual command across this owned process tree.
    for task in Path('/proc/'+str(pid)+'/task').iterdir():
     try: pending.extend(int(value) for value in (task/'children').read_text().split())
     except FileNotFoundError: pass
   except FileNotFoundError: pass
  return False
 def release_operation():
  release.write_text('release')
  if isinstance(held,sqlite3.Connection): held.rollback()
  elif held is not None: fcntl.flock(held.fileno(),fcntl.LOCK_UN)
 secondary=None; held=None
 with (root/'primary.stdout').open('xb') as stdout,(root/'primary.stderr').open('xb') as stderr:
  gui=subprocess.Popen([gtk,'--history','--config',str(a)],env=env,stdin=subprocess.DEVNULL,stdout=stdout,stderr=stderr)
  try:
   wait(lambda:any(n.name=='History on' and showing(n) for n in nodes()),'Initial unique GTK window did not load')
   assert primary_pid()==gui.pid
   if case=='replay':
    activate('Paste again'); activate('Paste dictation')
    wait(started.is_file,'Replay never reached the actual private pre-output hook')
   elif case=='search':
    indexed=subprocess.run([cli,'--config',str(a),'history','--json','reindex'],env=env,capture_output=True,text=True,timeout=10)
    assert indexed.returncode==0,indexed.stderr
    held=sqlite3.connect(root/'data/voxtype/meetings/index.db'); held.execute('BEGIN IMMEDIATE')
    search=wait(lambda:next((n for n in nodes() if n.getRoleName() in ['text','entry'] and showing(n) and n.getState().contains(pyatspi.STATE_EDITABLE)),None),'Missing search field')
    assert search.queryEditableText().setTextContents('synthetic')
    wait(lambda:blocked_command('search'),'Real history search command never reached the held database')
   else:
    held=(folder/'.lock').open('a+b'); fcntl.flock(held.fileno(),fcntl.LOCK_EX)
    activate('Delete'); activate('Delete dictation')
    wait(lambda:blocked_command('delete'),'Real deletion command never reached the held day store')
   assert not output_a.exists() and not release.exists()
   secondary=subprocess.Popen([gtk,'--history','--config',str(b)],env=env,stdin=subprocess.DEVNULL,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
   try: forwarded_stdout,forwarded_stderr=secondary.communicate(timeout=2)
   except subprocess.TimeoutExpired:
    raise AssertionError('GTK main loop is blocked by '+case+': normal forwarded launch did not return before releasing its private hook/storage lock') from None
   assert secondary.returncode==0,forwarded_stdout+forwarded_stderr
   assert not release.exists() and not output_a.exists(), 'Responsiveness must be observed while output is still waiting'
   wait(lambda:any(n.name=='History off' and showing(n) for n in nodes()),'Forwarded B window did not render while A '+case+' waited',timeout=2)
   assert primary_pid()==gui.pid
   release_operation()
   if case=='replay':
    wait(output_a.is_file,'Released replay did not finish')
    assert output_a.read_text()=='Synthetic slow replay.\n'
   else:
    wait(lambda:not blocked_command(case),'Released history command did not finish')
    assert not output_a.exists()
   assert not output_b.exists(), 'Opening B must never retarget A replay'
   state=subprocess.run([cli,'--config',str(a),'history','--json','status'],env=env,capture_output=True,text=True,timeout=10)
   assert state.returncode==0 and json.loads(state.stdout)['count']==(0 if case=='delete' else 1)
   print(json.dumps({'case':'normal-gtk-responsive-during-'+case,'primary_pid':gui.pid,'forwarded_launch_exit':secondary.returncode,'config_a_output':output_a.read_text() if output_a.exists() else None,'b_untouched':True,'new_history_entries':False,'scope':'private synthetic hook/file/storage locks and actual unique GTK/CLI; not desktop focus acceptance'}))
  finally:
   release_operation()
   if secondary is not None and secondary.poll() is None:
    try: secondary.communicate(timeout=5)
    except subprocess.TimeoutExpired: secondary.kill(); secondary.communicate(timeout=5)
   if gui.poll() is None:
    gui.terminate()
    try: gui.wait(timeout=5)
    except subprocess.TimeoutExpired: gui.kill(); gui.wait(timeout=5)
   if held is not None: held.close()
   print((root/'primary.stdout').read_text(errors='replace'))
   print((root/'primary.stderr').read_text(errors='replace'))
