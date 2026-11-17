"""Shared real UI acceptance for staged History -> ordinary saved Meetings.
Only called inside the private NAS compositor/session by the actual launcher tests.
"""
import datetime,json,os,signal,sqlite3,subprocess,time
from pathlib import Path
import pyatspi
from gi.repository import Gio,GLib

class MeetingLinks:
 def __init__(self,root,env,exe,configs):
  assert os.environ.get('HOME')=='/work/home' and os.environ.get('DBUS_SESSION_BUS_ADDRESS')
  self.root=root;self.env=env;self.exe=str(exe);self.configs=configs;self.saved={};self.records=[];self.primary=None
  self.id='11111111-2222-4333-8444-555555555555'
  self.failure_mode=os.environ.get('VOX_HISTORY_LINK_FAILURE_CASE','')
  assert self.failure_mode in ['', 'search-reopen','cross-context','copy-search']
  self.client_bin=self.exe
  self.bus=Gio.bus_get_sync(Gio.BusType.SESSION,None)
  for label,path in configs.items():
   store=root/('linked-store-'+label)
   path.write_text('[hotkey]\nenabled=false\n[history]\nenabled=true\n[meeting]\nview="gtk4"\nstorage_path='+json.dumps(str(store))+'\n[meeting.assistant]\nagent="private-unsupported-'+label+'"\n')
   if self.failure_mode:path.write_text(path.read_text().replace('view="gtk4"','view="none"'))
   self.cli(label,'meeting','list','--json')
   folder=store/'synthetic';folder.mkdir()
   title='Private linkedneedle '+label+' meeting';text='Private selected saved '+label+' transcript.'
   (folder/'transcript.json').write_text(json.dumps({'segments':[{'id':1,'start_ms':0,'end_ms':1000,'text':text,'source':'unknown','chunk_id':0}],'total_chunks':1}))
   now=int(datetime.datetime.now().timestamp())
   with sqlite3.connect(store/'index.db') as db:
    db.execute('INSERT INTO meetings(id,title,started_at,ended_at,duration_secs,status,chunk_count,storage_path,audio_retained) VALUES(?,?,?,?,?,?,?,?,?)',(self.id,title,now,now,1,'completed',1,str(folder),0))
   self.saved[label]=(title,text,path.read_bytes(),(folder/'transcript.json').read_bytes(),folder)
   self.cli(label,'history','reindex','--json')
  if self.failure_mode:
   wrapper=root/'canonical-observer';self.client_bin=str(wrapper)
   wrapper.write_text('#!/usr/bin/python3\nimport json,os,subprocess,sys,time\nfrom pathlib import Path\nr=Path(__file__).parent\na=sys.argv[1:]\nheld=("copy" in a and '+repr(self.failure_mode=='copy-search')+') or ("meeting" in a and "view" in a and '+repr(self.failure_mode!='copy-search')+')\nif held:\n (r/"action-held.json").write_text(json.dumps({"argv":a,"pid":os.getpid()}))\n deadline=time.monotonic()+15\n while not (r/"action-release").exists():\n  if time.monotonic()>deadline:raise SystemExit("Private action gate expired")\n  time.sleep(.02)\nresult=subprocess.run(['+repr(self.exe)+',*a])\nif held:(r/"action-finished.json").write_text(json.dumps({"rc":result.returncode,"argv":a}))\nraise SystemExit(result.returncode)\n');wrapper.chmod(0o500)
 def cli(self,label,*args):
  result=subprocess.run([self.exe,'--config',str(self.configs[label]),*args],env=self.env,capture_output=True,text=True,timeout=10)
  assert result.returncode==0,(args,result.stdout,result.stderr)
  return json.loads(result.stdout)
 def walk(self,node,depth=0):
  if depth>30:return
  try:count=node.childCount;node.name
  except GLib.Error:return
  yield node
  for i in range(count):
   try:child=node.getChildAtIndex(i)
   except GLib.Error:continue
   if child is not None:yield from self.walk(child,depth+1)
 def shown(self,node):
  try:return node.getState().contains(pyatspi.STATE_SHOWING)
  except GLib.Error:return False
 def all(self):return [n for n in self.walk(pyatspi.Registry.getDesktop(0)) if self.shown(n)]
 def frame(self,name):
  frames=[]
  for n in self.all():
   try:
    if n.getRoleName() in ['frame','window'] and n.name==name:frames.append(n)
   except GLib.Error:continue
  return frames[0] if len(frames)==1 else None
 def inside(self,name):
  frame=self.frame(name)
  return [n for n in self.walk(frame) if self.shown(n)] if frame is not None else []
 def wait(self,check,message):
  deadline=time.monotonic()+12
  while True:
   try:result=check()
   except GLib.Error:result=None
   if result:return result
   assert time.monotonic()<deadline,(message,[(n.name,n.getRoleName()) for n in self.all()])
   time.sleep(.05)
 def press(self,frame,name):
  button=self.wait(lambda:next((n for n in self.inside(frame) if n.name==name and n.getRoleName() in ['button','push button','toggle button'] and (frame!='Voxtype History' or n.getState().contains(pyatspi.STATE_ENABLED))),None),'Missing enabled real '+name+' in '+frame)
  print('ACTUAL_ACCESSIBLE_ACTION_STATES',frame,name,[str(state) for state in button.getState().getStates()],flush=True)
  assert button.queryAction().doAction(0),name
 def pid(self):return self.bus.call_sync('org.freedesktop.DBus','/org/freedesktop/DBus','org.freedesktop.DBus','GetConnectionUnixProcessID',GLib.Variant('(s)',('io.voxtype.Meetings',)),GLib.VariantType.new('(u)'),Gio.DBusCallFlags.NONE,1000,None).unpack()[0]
 def exercise(self,launch,surface):
  if self.failure_mode:
   self.failure_exercise(launch,surface)
   return
  try:
   for label in ['A','B','A','D','A']:
    launch(label)
    self.wait(lambda:self.frame('Voxtype History'),'Canonical packaged launcher must show actual QML History frame')
    editor=self.wait(lambda:next((n for n in self.inside('Voxtype History') if n.getRoleName() in ['entry','text'] and n.getState().contains(pyatspi.STATE_EDITABLE)),None),'Real QML search editor missing')
    # Search runs as the text changes; there is no Search button.
    assert editor.queryEditableText().setTextContents('linkedneedle')
    title,text,original,transcript,folder=self.saved[label]
    self.wait(lambda:any(title in n.name for n in self.inside('Voxtype History')),'Actual QML search must show the current private-store meeting')
    shot='/work/out/history-link-'+surface+'-'+label+'-search.png'
    subprocess.run(['grim','-o','HEADLESS-1',shot],env=self.env,check=True,timeout=5)
    self.press('Voxtype History','Open meeting')
    def selected():
     names=[n.name for n in self.inside('Meetings - Voxtype')]
     if not any(title in name for name in names) or not any(text in name for name in names):return False
     try:return self.pid()>1
     except GLib.Error:return False
    self.wait(selected,'Real ordinary Meetings frame must show selected private title/transcript, not History snippets')
    pid=self.pid()
    if self.primary is None:self.primary=pid
    assert pid==self.primary,'Keep ordinary Meetings single-window semantics across private A/B/A/default contexts'
    subprocess.run(['grim','-o','HEADLESS-1','/work/out/history-link-'+surface+'-'+label+'-selected.png'],env=self.env,check=True,timeout=5)
    actual=self.cli(label,'meeting','show',self.id,'--json')
    assert actual['metadata']['title']==title and actual['metadata']['status']=='completed' and not actual['metadata']['audio_retained']
    self.press('Meetings - Voxtype','Meetings')
    self.wait(lambda:any(n.name=='Recent meetings' for n in self.inside('Meetings - Voxtype')) and any(title in n.name for n in self.inside('Meetings - Voxtype')),'Real GUI Back action must list the selected private store')
    for other,(other_title,_,_,_,_) in self.saved.items():
     if other!=label:assert not any(other_title in n.name for n in self.inside('Meetings - Voxtype'))
    record={'label':label,'owner':pid,'title':title,'transcript':text,'saved_status':actual['metadata']['status'],'actual_GUI_store_list':True}
    self.records.append(record);print('ACTUAL_QML_LINK',surface,json.dumps(record),flush=True)
   for label,path in self.configs.items():
    title,text,original,transcript,folder=self.saved[label]
    assert path.read_bytes()==original and (folder/'transcript.json').read_bytes()==transcript
    assert not list(folder.glob('*.wav'))
   Path('/work/out/history-links-'+surface+'.json').write_text(json.dumps({'surface':surface,'actual_sequence':self.records,'config_and_saved_transcripts_unchanged':True,'boundary':'actual packaged launcher and UI controls on private Sway; not production/physical desktop'},indent=2))
  finally:
   try:pid=self.pid()
   except GLib.Error:pid=None
   if pid is not None:
    assert pid>1 and Path('/proc',str(pid)).stat().st_uid==os.getuid()
    os.kill(pid,signal.SIGTERM)

 def failure_exercise(self,launch,surface):
  launch('A')
  self.wait(lambda:self.frame('Voxtype History'),'Real packaged History frame missing')
  def search(query):
   editor=self.wait(lambda:next((n for n in self.inside('Voxtype History') if n.getRoleName() in ['entry','text'] and n.getState().contains(pyatspi.STATE_EDITABLE)),None),'Real search editor missing')
   assert editor.queryEditableText().setTextContents(query)
  query='Synthetic' if self.failure_mode=='copy-search' else 'linkedneedle'
  search(query)
  self.press('Voxtype History','Copy' if self.failure_mode=='copy-search' else 'Open meeting')
  held=self.wait(lambda:json.loads((self.root/'action-held.json').read_text()) if (self.root/'action-held.json').exists() else None,'Actual canonical child must reach private execution gate')
  assert held['argv'][:2]==['--config',str(self.configs['A'])]
  if self.failure_mode=='copy-search':
   entry=self.cli('A','history','list','--json')['entries'][0]['id']
   self.cli('A','history','delete',entry,'--json')
   reference=subprocess.run([self.exe,'--config',str(self.configs['A']),'history','--json','copy',entry],env=self.env,capture_output=True,text=True,timeout=10)
   assert reference.returncode!=0
   failure=json.loads(reference.stdout)['error']
  else:failure='No meeting view to open'
  search('private-unmatched-current-results')
  launch('A')  # Actual public host reopen also refreshes while the action waits.
  if self.failure_mode=='cross-context':
   launch('B');search('private-unmatched-current-results')
   self.wait(lambda:any(n.name=='No matches' for n in self.inside('Voxtype History')),'Independent B list must remain responsive while A launch waits')
  (self.root/'action-release').touch()
  finished=self.wait(lambda:json.loads((self.root/'action-finished.json').read_text()) if (self.root/'action-finished.json').exists() else None,'Actual canonical command must finish')
  assert finished['rc']!=0,finished
  if self.failure_mode=='cross-context':
   assert not any(failure in n.name for n in self.inside('Voxtype History')),'Hidden A error must not retarget B'
   launch('A')
  self.wait(lambda:any(n.name=='No matches' for n in self.inside('Voxtype History')),'Latest search results must remain current after action completion')
  print('ACTUAL_QML_FAILED_ACTION_BARRIER',json.dumps({'surface':surface,'mode':self.failure_mode,'held':held,'real_finished':finished,'expected_error':failure}),flush=True)
  subprocess.run(['grim','-o','HEADLESS-1','/work/out/history-action-'+surface+'-'+self.failure_mode+'.png'],env=self.env,check=True,timeout=5)
  self.wait(lambda:any(failure in n.name for n in self.inside('Voxtype History')),'Failed action completion must remain observable after search/refresh/reopen')
  for label,(_,_,original,transcript,folder) in self.saved.items():
   assert self.configs[label].read_bytes()==original and (folder/'transcript.json').read_bytes()==transcript
  record={'surface':surface,'mode':self.failure_mode,'actual_held_argv':held['argv'],'actual_canonical_exit':finished['rc'],'observable_error':failure,'current_results':'No matches','config_transcripts_unchanged':True}
  print('ACTUAL_QML_ACTION_LIFETIME',json.dumps(record),flush=True)
  Path('/work/out/history-action-'+surface+'-'+self.failure_mode+'.json').write_text(json.dumps(record,indent=2))
  subprocess.run(['grim','-o','HEADLESS-1','/work/out/history-action-'+surface+'-'+self.failure_mode+'.png'],env=self.env,check=True,timeout=5)
