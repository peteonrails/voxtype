"""Actual canonical GTK History search result -> selected saved meeting window.
Private NAS Xvfb/D-Bus only. No substitute launcher, provider or product I/O.
"""
import datetime,json,os,signal,sqlite3,subprocess,tempfile,time
from pathlib import Path
import pyatspi
from gi.repository import Gio,GLib
assert os.environ.get('HOME')=='/work/home'
assert os.environ.get('DISPLAY') and os.environ.get('DBUS_SESSION_BUS_ADDRESS')
assert not os.environ.get('WAYLAND_DISPLAY') and not os.environ.get('VOXTYPE_MEETINGS_GTK4_SMOKE')
authority=Path(os.environ['XAUTHORITY'])
assert authority.is_relative_to('/work/scratch') and authority.stat().st_uid==os.getuid()
os.umask(0o077)
exe=os.environ.get('VOXTYPE_HISTORY_TEST_BIN','/work/target/debug/voxtype')
bus=Gio.bus_get_sync(Gio.BusType.SESSION,None)
def owner(name):
 return bus.call_sync('org.freedesktop.DBus','/org/freedesktop/DBus','org.freedesktop.DBus','GetConnectionUnixProcessID',GLib.Variant('(s)',(name,)),GLib.VariantType.new('(u)'),Gio.DBusCallFlags.NONE,1000,None).unpack()[0]
def walk(node,depth=0):
 if depth>30:return
 try:count=node.childCount;node.name
 except GLib.Error:return
 yield node
 for i in range(count):
  try:child=node.getChildAtIndex(i)
  except GLib.Error:continue
  if child is not None:yield from walk(child,depth+1)
def nodes():
 result=[]
 for node in walk(pyatspi.Registry.getDesktop(0)):
  try:
   if node.getState().contains(pyatspi.STATE_SHOWING):result.append(node)
  except GLib.Error:pass
 return result
def wait(check,message):
 deadline=time.monotonic()+8
 while True:
  result=check()
  if result:return result
  assert time.monotonic()<deadline,(message,[(n.name,n.getRoleName()) for n in nodes()])
  time.sleep(.05)
def capture(path):
 from PIL import ImageGrab
 ImageGrab.grab(xdisplay=os.environ['DISPLAY']).save(path)
with tempfile.TemporaryDirectory(dir='/work/scratch',prefix='history-meeting-link-') as name:
 root=Path(name);runtime=root/'runtime';runtime.mkdir(mode=0o700)
 env=dict(os.environ,HOME=str(root),XDG_CONFIG_HOME=str(root/'config'),XDG_DATA_HOME=str(root/'data'),XDG_RUNTIME_DIR=str(runtime),GDK_BACKEND='x11',GSK_RENDERER='cairo',GTK_A11Y='atspi')
 config=root/'config.toml';store=root/'meetings'
 config.write_text('[hotkey]\nenabled=false\n[history]\nenabled=true\n[meeting]\nview="gtk4"\nstorage_path='+json.dumps(str(store))+'\n')
 def cli(*args):
  result=subprocess.run([exe,'--config',str(config),*args],env=env,capture_output=True,text=True,timeout=10)
  print('ACTUAL_CANONICAL',args,result.returncode,result.stdout,result.stderr,flush=True)
  assert result.returncode==0,result.stderr
  return result.stdout
 meeting='11111111-2222-4333-8444-555555555555';title='Private linked invoice meeting';text='Private selected saved invoice transcript.'
 cli('meeting','list','--json')
 saved=store/'synthetic';saved.mkdir()
 (saved/'transcript.json').write_text(json.dumps({'segments':[{'id':1,'start_ms':0,'end_ms':1000,'text':text,'source':'unknown','chunk_id':0}],'total_chunks':1}))
 now=int(datetime.datetime.now().timestamp())
 with sqlite3.connect(store/'index.db') as db:
  db.execute('INSERT INTO meetings(id,title,started_at,ended_at,duration_secs,status,chunk_count,storage_path,audio_retained) VALUES(?,?,?,?,?,?,?,?,?)',(meeting,title,now,now,1,'completed',1,str(saved),0))
 cli('history','reindex','--json')
 hits=json.loads(cli('history','search','invoice','--json'))
 assert any(hit.get('source')=='meeting' and hit.get('meeting_id')==meeting for hit in hits),hits
 try:
  cli('history','view')
  wait(lambda:any(n.name=='History on' for n in nodes()),'Actual History window must load')
  search=wait(lambda:next((n for n in nodes() if n.getRoleName() in ['entry','text'] and n.getState().contains(pyatspi.STATE_EDITABLE)),None),'Actual History search must be editable')
  assert search.queryEditableText().setTextContents('invoice')
  def meeting_row():
   # A meeting is a row tagged Meeting, with its title and its own action.
   for item in nodes():
    if item.getRoleName()!='list item':continue
    row=[n for n in walk(item) if n.getState().contains(pyatspi.STATE_SHOWING)]
    names=[n.name for n in row]
    if 'Meeting' in names and title in names:return row
  wait(meeting_row,'Actual unified search must render saved meeting result')
  capture('/work/out/history-meeting-link-before.png')
  button=wait(lambda:next((n for n in meeting_row() or [] if n.name=='Open meeting' and n.getRoleName() in ['button','push button']),None),'Meeting search result must provide its real Open meeting action')
  assert button.queryAction().doAction(0)
  def selected_meeting():
   frames=[n for n in nodes() if n.getRoleName()=='frame' and n.name=='Meetings - Voxtype']
   if len(frames)!=1:return False
   # Search snippets can equal the saved text: only the actual ordinary
   # Meetings frame is authoritative, not the initiating History results.
   labels=[n.name for n in walk(frames[0]) if n.getState().contains(pyatspi.STATE_SHOWING)]
   if not any(title in value for value in labels) or not any(text in value for value in labels):return False
   try:return owner('io.voxtype.Meetings')>1
   except GLib.Error:return False
  wait(selected_meeting,'Canonical GUI action must render selected saved title and transcript inside its actual Meetings frame')
  capture('/work/out/history-meeting-link-selected.png')
  print(json.dumps({'actual_history_meeting_link':'passed','meeting':meeting,'canonical_saved':json.loads(cli('meeting','show',meeting,'--json'))}),flush=True)
 finally:
  for app in ['io.voxtype.History','io.voxtype.Meetings']:
   try:pid=owner(app)
   except GLib.Error:continue
   assert pid>1 and Path('/proc',str(pid)).stat().st_uid==os.getuid()
   os.kill(pid,signal.SIGTERM)
