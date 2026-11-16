"""Real GTK forwarding with a controlled private meeting.sock protocol feed.
The producer implements inspected events.rs hello/status/error frames only.
A transparent argv recorder execs the byte-identical real CLI; no fake replies.
"""
import datetime,hashlib,json,os,shutil,signal,socket,sqlite3,subprocess,tempfile,threading,time
from pathlib import Path
import pyatspi
from history_meeting_links import MeetingLinks
assert os.environ.get('HOME')=='/work/home' and os.environ.get('DISPLAY') and os.environ.get('DBUS_SESSION_BUS_ADDRESS')
auth=Path(os.environ['XAUTHORITY']);assert auth.is_relative_to('/work/scratch') and auth.stat().st_uid==os.getuid()
os.umask(0o077)
case=os.environ['VOX_MEETING_LIVE_CASE'];assert case in ['saved-recording','saved-paused','ask-canonical','ask-direct','ask-idle','ask-generation','tab-notes','tab-ask','tab-idle','ask-id-canonical','ask-id-direct','ask-id-missing','ask-id-idle','ask-no-feed','ask-no-feed-direct','tab-no-feed','ask-id-no-feed','reconnect-recording','reconnect-paused'] or case in ['lifetime-saved-reconnect','lifetime-saved-manual','lifetime-worker-reconnect','lifetime-worker-manual','lifetime-worker-same','lifetime-rename-reconnect','lifetime-rename-same','lifetime-move-reconnect','lifetime-note-reconnect','lifetime-stop-reconnect']
no_feed=case in ['ask-no-feed','ask-no-feed-direct','tab-no-feed','ask-id-no-feed']
exe=Path(os.environ.get('VOXTYPE_HISTORY_TEST_BIN','/work/target/debug/voxtype'));gtk=Path(os.environ.get('VOXTYPE_HISTORY_TEST_GTK_BIN','/work/target/debug/voxtype-meetings-gtk4'))
x='33333333-2222-4333-8444-555555555555';z='44444444-2222-4333-8444-555555555555'
with tempfile.TemporaryDirectory(dir='/work/scratch',prefix='meeting-live-') as name:
 root=Path(name);runtime=root/'runtime';runtime.mkdir(mode=0o700);(runtime/'voxtype').mkdir(mode=0o700)
 env=dict(os.environ,HOME=str(root),XDG_CONFIG_HOME=str(root/'config'),XDG_DATA_HOME=str(root/'data'),XDG_RUNTIME_DIR=str(runtime),GDK_BACKEND='x11',GSK_RENDERER='cairo',GTK_A11Y='atspi')
 configs={}
 for label in ['A','B']:
  (root/label).mkdir();configs[label]=root/label/'config.toml'
 ui=MeetingLinks(root,env,exe,configs)
 for label,config in configs.items():
  store=root/('linked-store-'+label);folder=store/'active';folder.mkdir()
  (folder/'transcript.json').write_text(json.dumps({'segments':[{'id':1,'start_ms':0,'end_ms':1000,'text':'Stored active '+label,'source':'unknown','chunk_id':0}],'total_chunks':1}))
  (folder/'notes.md').write_text('Private saved notes '+label)
  (folder/'asks.jsonl').write_text(json.dumps({'id':'private-saved-question','time':datetime.datetime.now(datetime.timezone.utc).isoformat(),'question':'Private recorded question '+label,'answer':'Private recorded answer '+label,'agent':'claude','agent_name':'Synthetic saved record','citations':[]})+'\n')
  with sqlite3.connect(store/'index.db') as db:db.execute('INSERT INTO meetings(id,title,started_at,duration_secs,status,chunk_count,storage_path,audio_retained) VALUES(?,?,?,?,?,?,?,?)',(x,'Stored active '+label,int(time.time()),1,'recording',1,str(folder),0))
 binaries=root/'bin';binaries.mkdir(mode=0o700)
 for source,destination in [(exe,binaries/'voxtype-real'),(gtk,binaries/'voxtype-meetings-gtk4')]:
  shutil.copyfile(source,destination);destination.chmod(0o500);assert hashlib.sha256(source.read_bytes()).digest()==hashlib.sha256(destination.read_bytes()).digest()
 trace=root/'actual-argv.jsonl';wrapper=binaries/'voxtype'
 real=str(binaries/'voxtype-real');helper=str(binaries/'voxtype-meetings-gtk4')
 if case.startswith('lifetime-'):
  from meeting_view_lifetime import install_observer
  install_observer(root,wrapper,real)
 else:
  wrapper.write_text('#!/usr/bin/python3\nimport json,os,sys\nfrom pathlib import Path\nr=Path(__file__).parent\nwith (r.parent/"actual-argv.jsonl").open("a") as f:f.write(json.dumps(sys.argv[1:])+"\\n")\nos.execv(str(r/"voxtype-real"),[str(r/"voxtype-real"),*sys.argv[1:]])\n');wrapper.chmod(0o500)
 listener=socket.socket(socket.AF_UNIX);listener.bind(str(runtime/'voxtype/meeting.sock'));listener.listen();listener.settimeout(.2)
 slots=[];stop=threading.Event();status='idle' if case in ['ask-idle','ask-id-idle','tab-idle'] else 'paused' if case.endswith('paused') else 'recording'
 snapshot={'event':'hello','status':status,'meeting_id':None if status=='idle' else x,'title':'Private active X','started_at':datetime.datetime.now(datetime.timezone.utc).isoformat(),'chunk_count':1,'diarization':{'enabled':False},'segments':[] if status=='idle' else [{'id':1,'start_ms':0,'end_ms':1000,'text':'Private live X transcript.','source':'unknown','chunk_id':0}],'recording_notice':False}
 def send(sock,events):
  try:sock.sendall((''.join(json.dumps(e)+'\n' for e in events)).encode())
  except (BrokenPipeError,ConnectionResetError,OSError):pass
 def serve_client(slot):
  if not slot['release'].wait(6):return
  marker='Private processed feed '+str(slot['number'])
  hello=dict(snapshot)
  send(slot['socket'],[hello,*[{'event':'error','meeting_id':mid,'message':marker} for mid in [x,z,ui.id]]])
 def accept():
  while not stop.is_set():
   try:client,_=listener.accept()
   except socket.timeout:continue
   except OSError:return
   slot={'socket':client,'release':threading.Event(),'number':len(slots)+1};slots.append(slot)
   if slot['number']==1 and not no_feed:slot['release'].set()
   threading.Thread(target=serve_client,args=(slot,),daemon=True).start()
 threading.Thread(target=accept,daemon=True).start()
 def names():return [n.name for n in ui.inside('Meetings - Voxtype')]
 def visible(text):return any(text in n for n in names())
 def launch(label,meeting=None,question=None,direct=False,tab=None,ask_id=None):
  command=[helper,'--config',str(configs[label])] if direct else [real,'--config',str(configs[label]),'meeting','view']
  if meeting is not None:command+=['--meeting',meeting] if direct else [meeting]
  if question is not None:command+=['--ask',question]
  if tab is not None:command+=['--tab',tab]
  if ask_id is not None:command+=['--ask-id',ask_id]
  result=subprocess.run(command,env=env,capture_output=True,text=True,timeout=8)
  print('ACTUAL_PRIVATE_LIVE_LAUNCH',command,result.returncode,result.stdout,result.stderr,flush=True);assert result.returncode==0,result.stderr
 def next_slot(count):return ui.wait(lambda:slots[count] if len(slots)>count else None,'Actual new GTK feed connection missing')
 def barrier(slot):
  slot['release'].set();ui.wait(lambda:visible('Private processed feed '+str(slot['number'])),'Actual viewer must consume released hello and subsequent named error barrier')
 def recorded():return [json.loads(line) for line in trace.read_text().splitlines()] if trace.exists() else []
 records=[]
 try:
  launch('A')
  ui.wait(lambda:visible('Recent meetings') if status=='idle' or no_feed else visible('Private active X'),'Initial actual live/list view missing')
  first=next_slot(0);primary=ui.pid()
  if case.startswith('lifetime-'):
   from meeting_view_lifetime import exercise
   records.append(exercise(locals()))
  elif case.startswith('saved-') or case.startswith('reconnect-'):
   for label in ['A','B','A']:
    count=len(slots);launch(label,ui.id);slot=next_slot(count)
    title,text,*_=ui.saved[label]
    ui.wait(lambda:visible(title) and visible(text),'Explicit saved Y must initially be selected')
    barrier(slot)
    assert visible(title) and visible(text) and not visible('Private live X transcript.'),('Initial snapshot must not replace requested saved Y with active X',label,names())
    records.append({'label':label,'snapshot':status,'saved_title':title,'barrier':slot['number']})
   if case.startswith('reconnect-'):
    count=len(slots);slots[-1]['socket'].shutdown(socket.SHUT_RDWR);slots[-1]['socket'].close();slot=next_slot(count);barrier(slot)
    assert visible(title) and visible(text),'Same-ID reconnect must retain explicit saved Y'
   snapshot.update(status=status,meeting_id=z,title='Private genuinely new Z',segments=[{'id':1,'start_ms':0,'end_ms':1000,'text':'Private genuinely new Z transcript.','source':'unknown','chunk_id':0}])
   count=len(slots)
   if case.startswith('reconnect-'):
    slots[-1]['socket'].shutdown(socket.SHUT_RDWR);slots[-1]['socket'].close()
   else:send(slots[-1]['socket'],[{'event':'status','status':'idle','meeting_id':x,'recording_notice':False},{'event':'status','status':'recording','meeting_id':z,'recording_notice':False}])
   slot=next_slot(count);barrier(slot)
   ui.wait(lambda:visible('Private genuinely new Z transcript.'),'Genuinely new live meeting must still take over saved view')
  elif no_feed:
   count=len(slots);began=time.monotonic();launch('A',question=None if case in ['tab-no-feed','ask-id-no-feed'] else 'Private no-feed question' if case.endswith('direct') else 'actions',direct=case.endswith('direct'),tab='notes' if case=='tab-no-feed' else None,ask_id='private-saved-question' if case=='ask-id-no-feed' else None);slot=next_slot(count)
   ui.wait(lambda:visible('Meeting status is unavailable'),'No valid hello must fail visibly within the bounded request deadline')
   assert time.monotonic()-began<8 and not any('ask' in a for a in recorded())
   barrier(slot)
   ui.wait(lambda:visible('Private live X transcript.'),'Late real hello should update live state without replaying the failed question')
   assert not any('ask' in a for a in recorded())
   records.append({'deadline_feedback':'unavailable','late_hello_did_not_resurrect_question':True})
  elif case.startswith('tab-') or case.startswith('ask-id-'):
   for label in ['A','B','A'] if status!='idle' and case!='ask-id-missing' else ['A']:
    count=len(slots)
    launch(label,tab=('ask' if case=='tab-ask' else 'notes') if case.startswith('tab-') else None,ask_id=None if case.startswith('tab-') else 'absent-private-record' if case=='ask-id-missing' else 'private-saved-question',direct=case=='ask-id-direct')
    slot=next_slot(count);slot['release'].set()
    if status=='idle':
     ui.wait(lambda:visible('No active meeting'),'Deferred non-question launch with idle hello must give truthful feedback')
    elif case=='ask-id-missing':
     ui.wait(lambda:visible('Could not find saved question'),'Missing saved ask must not silently show an empty answer or waiting status')
    else:
     barrier(slot)
     def ask_focused():
      return any(node.getRoleName() in ['text','entry'] and node.getState().contains(pyatspi.STATE_SHOWING) and node.getState().contains(pyatspi.STATE_FOCUSED) and node.getState().contains(pyatspi.STATE_EDITABLE) and node.queryText().getText(0,-1)=='' for node in ui.inside('Meetings - Voxtype'))
     if case.startswith('ask-id-'):
      ui.wait(lambda:visible('Private recorded question '+label) and visible('Private recorded answer '+label),'Implicit saved answer must come from current private config/live ID')
      ui.wait(ask_focused,'Saved Ask launch must focus the actual question entry')
     elif case=='tab-ask':
      ui.wait(ask_focused,'Ask tab must focus the actual question entry without injecting a question')
     else:
      def note_loaded():
       for node in ui.inside('Meetings - Voxtype'):
        if node.getRoleName() in ['text','entry'] and 'Text' in node.get_interfaces() and node.queryText().getText(0,-1)=='Private saved notes '+label and node.getState().contains(pyatspi.STATE_FOCUSED):return True
       return False
      ui.wait(note_loaded,'Deferred notes tab must load current config/live-ID notes')
     assert not visible('Waiting for the current meeting status'),('Resolved non-question launch must clear waiting feedback',label,names())
    assert not any('ask' in a for a in recorded()),'Reopening notes/a saved answer must never run an agent'
    records.append({'label':label,'resolved_deferred_contract':case})
  elif case=='ask-idle':
   count=len(slots);launch('A',question='actions');slot=next_slot(count);slot['release'].set()
   ui.wait(lambda:visible('No active meeting'),'Implicit Ask with idle hello must give truthful visible feedback')
   assert not any('ask' in a for a in recorded())
  else:
   sequence=['A','B','A'] if case!='ask-generation' else ['B']
   if case=='ask-generation':
    count=len(slots);launch('A',question='actions');obsolete=next_slot(count)
   for label in sequence:
    question='Private literal '+label if case=='ask-direct' else 'What are the action items so far?'
    count=len(slots);launch(label,question=question if case=='ask-direct' else 'actions',direct=case=='ask-direct');slot=next_slot(count)
    if case=='ask-generation':obsolete['release'].set()
    slot['release'].set()
    expected=['--config',str(configs[label]),'meeting','ask',x,question,'--json']
    actual=ui.wait(lambda:next((a for a in recorded() if a==expected),None),'Implicit Ask must run actual canonical worker with current config, live ID and unchanged question')
    ui.wait(lambda:visible(question) and visible('private-unsupported-'+label),'Actual worker result/question must be rendered in selected viewer')
    assert ui.pid()==primary
    records.append({'label':label,'actual_worker_argv':actual})
   if case=='ask-generation':assert not any(a[:2]==['--config',str(configs['A'])] and 'ask' in a for a in recorded()),recorded()
  from PIL import ImageGrab
  ImageGrab.grab(xdisplay=env['DISPLAY']).save('/work/out/meeting-live-'+case+'.png')
  Path('/work/out/meeting-live-'+case+'.json').write_text(json.dumps({'case':case,'passed':True,'records':records,'actual_argv':recorded(),'schema':'events.rs hello/status/error; no audio/daemon/control requests'},indent=2))
 finally:
  if case.startswith('lifetime-'):
   from meeting_view_lifetime import diagnostics
   diagnostics(locals())
  from PIL import ImageGrab
  ImageGrab.grab(xdisplay=env['DISPLAY']).save('/work/out/meeting-live-'+case+'.png')
  print('ACTUAL_LIVE_FORWARDING_FINAL',json.dumps({'case':case,'feed_connections':len(slots),'actual_worker_argv':recorded(),'actual_frame_labels':names()}),flush=True)
  stop.set();listener.close()
  for slot in slots:slot['release'].set();slot['socket'].close()
  try:pid=ui.pid()
  except Exception:pid=None
  if pid is not None:assert pid>1 and Path('/proc',str(pid)).stat().st_uid==os.getuid();os.kill(pid,signal.SIGTERM)
