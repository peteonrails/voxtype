"""Dedicated real Done/Labeled completion witnesses; no product monkeypatches."""
import base64,hashlib,json,os,sqlite3,subprocess,time
from pathlib import Path
import pyatspi

KINDS=['rename','notes']
TRANSITIONS=['current','same-reconnect','manual','live','reconnect','context','return']

def install_observer(root,wrapper,real):
    assert not wrapper.exists()
    code=r'''#!/usr/bin/python3
import base64,hashlib,json,os,subprocess,sys,time
from pathlib import Path
r=Path(__file__).parent.parent;a=sys.argv[1:]
with (r/'actual-argv.jsonl').open('a') as f:f.write(json.dumps(a)+'\n')
kind=os.environ['VOX_MEETING_COMPLETION_CASE'].split(':')[0]
verb='label' if kind=='rename' else 'notes'
if len(a)<5 or a[2:4]!=['meeting',verb]:os.execv(REAL,[REAL,*a])
stdin=sys.stdin.buffer.read() if verb=='notes' else None
native=Path(REAL);before=hashlib.sha256(native.read_bytes()).hexdigest()
p=subprocess.Popen([REAL,*a],stdin=subprocess.PIPE if stdin is not None else subprocess.DEVNULL,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
stdout,stderr=p.communicate(stdin,timeout=10)
assert hashlib.sha256(native.read_bytes()).hexdigest()==before
record={'argv':a,'wrapper_pid':os.getpid(),'native_pid':p.pid,'uid':os.getuid(),'native_sha256':before,'rc':p.returncode,'stdin':None if stdin is None else base64.b64encode(stdin).decode(),'stdout':base64.b64encode(stdout).decode(),'stderr':base64.b64encode(stderr).decode()}
tmp=r/'completion-native.tmp';tmp.write_text(json.dumps(record));tmp.replace(r/'completion-native.json')
deadline=time.monotonic()+45
while not (r/'completion-release').exists():
 if time.monotonic()>deadline:raise SystemExit('Private completion barrier expired')
 time.sleep(.01)
sys.stdout.buffer.write(stdout);sys.stdout.buffer.flush();sys.stderr.buffer.write(stderr);sys.stderr.buffer.flush()
(r/'completion-forwarded.json').write_text(json.dumps({'rc':p.returncode,'stdout_sha256':hashlib.sha256(stdout).hexdigest(),'stderr_sha256':hashlib.sha256(stderr).hexdigest()}))
raise SystemExit(p.returncode)
'''.replace('REAL',repr(real))
    wrapper.write_text(code);wrapper.chmod(0o500)
    (root/'completion-observer.json').write_text(json.dumps({'sha256':hashlib.sha256(wrapper.read_bytes()).hexdigest(),'native_sha256':hashlib.sha256(Path(real).read_bytes()).hexdigest(),'mode':oct(wrapper.stat().st_mode&0o777),'source':'real Popen/communicate, exact passthrough bytes and genuine status, bounded post-completion delivery only'}))

def stores(root):
    result={}
    for label in ['A','B']:
        store=root/('linked-store-'+label)
        files={str(p.relative_to(store)):hashlib.sha256(p.read_bytes()).hexdigest() for p in store.rglob('*') if p.is_file() and not p.name.startswith('index.db')}
        with sqlite3.connect(store/'index.db') as db:rows=db.execute('SELECT * FROM meetings ORDER BY id').fetchall()
        result[label]={'files':files,'index_rows':rows}
    return result

def exercise(c):
    ui,root,env,x,z,snapshot,slots=(c[k] for k in ['ui','root','env','x','z','snapshot','slots'])
    launch,next_slot,barrier,visible,recorded=(c[k] for k in ['launch','next_slot','barrier','visible','recorded'])
    case=os.environ['VOX_MEETING_COMPLETION_CASE'];kind,transition=case.split(':');assert kind in KINDS and transition in TRANSITIONS
    pid=ui.pid();cfg=c['configs']['A'];store=root/'linked-store-A'
    folder=store/'new-live';folder.mkdir();(folder/'notes.md').write_text('Private Z unchanged notes')
    (folder/'transcript.json').write_text(json.dumps({'segments':[{'id':1,'start_ms':0,'end_ms':1000,'text':'Private Z persisted transcript.','source':'unknown','chunk_id':0}],'total_chunks':1}))
    with sqlite3.connect(store/'index.db') as db:db.execute('INSERT INTO meetings(id,title,started_at,duration_secs,status,chunk_count,storage_path,audio_retained) VALUES(?,?,?,?,?,?,?,?)',(z,'Stored Z',int(time.time()),1,'recording',1,str(folder),0))
    # Replace generation-zero startup once, before creating the command. No
    # context replacement is used as a shortcut for manual/live/return cases.
    count=len(slots);launch('A');barrier(next_slot(count))
    ui.wait(lambda:visible('Private live X transcript.'),'Actual initial X view missing')
    before=stores(root);c['completion_before_native']=before
    value='Private completed X speaker' if kind=='rename' else 'Private completed notes X'
    if kind=='rename':
        ui.press('Meetings - Voxtype','● Unknown (1)');ui.wait(lambda:ui.frame('Rename Unknown'),'Real rename dialog missing')
        entry=ui.wait(lambda:next((n for n in ui.inside('Rename Unknown') if n.getRoleName() in ['text','entry'] and n.getState().contains(pyatspi.STATE_EDITABLE)),None),'Actual rename editor missing')
        assert entry.queryEditableText().setTextContents(value);ui.press('Rename Unknown','OK')
        expected=['--config',str(cfg),'meeting','label',x,'Unknown',value];feedback='Rename: done'
    else:
        entry=ui.wait(lambda:next((n for n in ui.inside('Meetings - Voxtype') if n.getRoleName()=='text' and 'Text' in n.get_interfaces() and n.queryText().getText(0,-1)=='Private saved notes A'),None),'Actual X notes editor missing')
        assert entry.queryEditableText().setTextContents(value);ui.press('Meetings - Voxtype','Save notes')
        expected=['--config',str(cfg),'meeting','notes',x,'--set-file','-'];feedback='Notes: done'
    record=ui.wait(lambda:json.loads((root/'completion-native.json').read_text()) if (root/'completion-native.json').exists() else None,'Genuine native command must complete before delivery is held')
    assert record['argv']==expected and record['rc']==0 and record['uid']==os.getuid()==65532,record
    assert record['native_sha256']==hashlib.sha256(Path(c['real']).read_bytes()).hexdigest()
    if kind=='notes':assert base64.b64decode(record['stdin'])==value.encode()
    def owner():
        for task in Path('/proc',str(pid),'task').iterdir():
            try:
                if str(record['wrapper_pid']) in (task/'children').read_text().split():return task
            except FileNotFoundError:pass
    task=ui.wait(owner,'Actual GTK command worker must remain behind the delivery barrier')
    assert not visible(feedback),'No fabricated completion before real exit delivery'
    if kind=='rename':
        transcript=json.loads((store/'active/transcript.json').read_text());assert transcript['segments'][0]['speaker_label']==value,transcript
        assert not visible(value),'Labeled reply must actually remain withheld from live view'
    else:assert (store/'active/notes.md').read_text().strip()==value
    after_native=stores(root);assert after_native['B']==before['B'];assert after_native['A']!=before['A']
    c['completion_after_native']=after_native
    current=x;title='Private active X';text='Private live X transcript.'
    def row(index,expected_label):
        # The frozen222 runtime lacks row Action/Focus and reports0,0 positions.
        # These exact private980x680 first/two stored-row coordinates are checked
        # against actual labels and the subsequent rendered target, not internals.
        node=ui.wait(lambda:next((n for n in ui.inside('Meetings - Voxtype') if n.getRoleName()=='list item' and any(expected_label in child.name for child in ui.walk(n))),None),'Real list row missing '+expected_label)
        rows=[n for n in ui.inside('Meetings - Voxtype') if n.getRoleName()=='list item']
        actual=next(i for i,n in enumerate(rows) if any(expected_label in child.name for child in ui.walk(n)))
        if index is not None:assert actual==index
        index=actual
        windows=subprocess.check_output(['xdotool','search','--onlyvisible','--pid',str(pid),'--name','^Meetings - Voxtype$'],env=env,text=True).splitlines();assert len(windows)==1
        geometry=dict(line.split('=',1) for line in subprocess.check_output(['xdotool','getwindowgeometry','--shell',windows[0]],env=env,text=True).splitlines());assert (geometry['WIDTH'],geometry['HEIGHT'])==('980','680')
        subprocess.run(['xdotool','windowfocus',windows[0],'mousemove','--window',windows[0],'150',str(145+index*29),'click','--repeat','2','--delay','100','1'],env=env,check=True,timeout=5)
    if transition in ['manual','return']:
        ui.press('Meetings - Voxtype','Meetings');ui.wait(lambda:visible('Recent meetings'),'Back must show actual list')
        if transition=='manual':
            current=ui.id;title,text,*_=ui.saved['A'];row(None,title)
            ui.wait(lambda:visible(title) and visible(text) and visible('Past meeting: read-only'),'Manual selection must show real saved Y')
        else:
            row(0,'Private active X');ui.wait(lambda:visible(text) and not visible('Recent meetings'),'Same-ID return must actually reopen live X')
    elif transition=='context':
        count=len(slots);launch('B',meeting=ui.id);barrier(next_slot(count));current=ui.id;title,text,*_=ui.saved['B'];ui.wait(lambda:visible(title) and visible(text),'Actual B store selection missing')
    elif transition in ['live','reconnect']:
        snapshot.update(meeting_id=z,title='Private genuinely new Z',segments=[{'id':1,'start_ms':0,'end_ms':1000,'text':'Private genuinely new Z transcript.','source':'unknown','chunk_id':0}]);count=len(slots)
        if transition=='reconnect':slots[-1]['socket'].shutdown(c['socket'].SHUT_RDWR);slots[-1]['socket'].close()
        else:c['send'](slots[-1]['socket'],[{'event':'status','status':'idle','meeting_id':x,'recording_notice':False},{'event':'status','status':'recording','meeting_id':z,'recording_notice':False}])
        barrier(next_slot(count));current=z;title='Private genuinely new Z';text='Private genuinely new Z transcript.';ui.wait(lambda:visible(title) and visible(text),'New live Z must actually render')
    elif transition=='same-reconnect':
        count=len(slots);slots[-1]['socket'].shutdown(c['socket'].SHUT_RDWR);slots[-1]['socket'].close();barrier(next_slot(count))
    ui.wait(lambda:visible(title) and visible(text),'Authoritative target content missing before release')
    baseline=stores(root);configs={k:p.read_bytes() for k,p in c['configs'].items()};c['completion_before_release']=baseline
    (root/'completion-release').touch();ui.wait(lambda:not task.exists(),'Actual GTK command thread must finish after genuine stdout/status release')
    forwarded=ui.wait(lambda:json.loads((root/'completion-forwarded.json').read_text()) if (root/'completion-forwarded.json').exists() else None,'Actual passthrough bytes were not delivered')
    assert forwarded['rc']==record['rc'] and forwarded['stdout_sha256']==hashlib.sha256(base64.b64decode(record['stdout'])).hexdigest() and forwarded['stderr_sha256']==hashlib.sha256(base64.b64decode(record['stderr'])).hexdigest()
    marker='Private completion processed '+case;c['send'](slots[-1]['socket'],[{'event':'error','meeting_id':current,'message':marker}]);ui.wait(lambda:visible(marker),'Actual GTK loop must cross post-worker completion barrier')
    valid=transition in ['current','same-reconnect']
    if valid:
        ui.wait(lambda:visible(feedback),'Valid current completion must give actual feedback')
        if kind=='rename':ui.wait(lambda:visible('● '+value+' (1)'),'Valid Labeled result must update actual live speaker chip')
    else:
        assert not visible(feedback),'Stale Done must not replace current view feedback'
        assert not visible('● '+value+' (1)'),'Stale Labeled must not rename current view speaker'
    assert visible(title) and visible(text),'Completion must not replace current selected content'
    assert stores(root)==baseline,'Delivery of old completion must not rewrite any private persisted metadata/index'
    assert all(p.read_bytes()==configs[k] for k,p in c['configs'].items())
    assert recorded()==[expected],recorded()
    result=dict(case=case,passed=True,old_target=x,current_target=current,original_config=str(cfg),actual_native=record,forwarded=forwarded,valid_current_control=valid,before_native=before,after_native=after_native,before_release=baseline,after_release=stores(root),labels=[n.name for n in ui.inside('Meetings - Voxtype')],boundary='real native completion, original persisted target, private post-exit delivery barrier; no fake product responses or inference')
    c['completion_result']=result;print('ACTUAL_NONASK_COMPLETION',json.dumps(result),flush=True);return result

def diagnostics(c):
    root=c['root'];case=os.environ['VOX_MEETING_COMPLETION_CASE'];(root/'completion-release').touch()
    record={p.name:json.loads(p.read_text()) for p in root.glob('completion-*.json')}
    result=dict(case=case,records=record,before_native=c.get('completion_before_native'),after_native=c.get('completion_after_native'),before_release=c.get('completion_before_release'),after=stores(root),result=c.get('completion_result'))
    label=case.replace(':','-');Path('/work/out/meeting-completion-'+label+'.json').write_text(json.dumps(result,indent=2));print('PRIVATE_NONASK_DIAGNOSTICS',json.dumps(result),flush=True)
    from PIL import ImageGrab
    ImageGrab.grab(xdisplay=c['env']['DISPLAY']).save('/work/out/meeting-completion-'+label+'.png')
