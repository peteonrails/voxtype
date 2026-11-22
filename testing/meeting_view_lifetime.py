"""Selected-meeting authority journeys using actual private GTK/canonical CLI."""
import base64,datetime,json,os,sqlite3,subprocess,time
from pathlib import Path
import pyatspi

def install_observer(root,wrapper,real):
    wrapper.write_text('''#!/usr/bin/python3
import base64,json,os,subprocess,sys,time
from pathlib import Path
r=Path(__file__).parent.parent;a=sys.argv[1:]
with (r/'actual-argv.jsonl').open('a') as f:f.write(json.dumps(a)+'\\n')
if 'ask' not in a:os.execv(REAL,[REAL,*a])
q=a[a.index('ask')+2];key='Y' if q=='Private held Y question' else 'Z'
p=subprocess.run([REAL,*a],capture_output=True)
(r/('worker-'+key+'.json')).write_text(json.dumps({'argv':a,'pid':os.getpid(),'rc':p.returncode,'stdout':base64.b64encode(p.stdout).decode(),'stderr':base64.b64encode(p.stderr).decode()}))
deadline=time.monotonic()+25
while not (r/('release-'+key)).exists():
 if time.monotonic()>deadline:raise SystemExit('Private worker gate expired')
 time.sleep(.01)
sys.stdout.buffer.write(p.stdout);sys.stdout.buffer.flush();sys.stderr.buffer.write(p.stderr);sys.stderr.buffer.flush()
raise SystemExit(p.returncode)
'''.replace('REAL',repr(real)))
    wrapper.chmod(0o500)

def exercise(c):
    ui,root,env,case,x,z,snapshot,slots=(c[k] for k in ['ui','root','env','case','x','z','snapshot','slots'])
    launch,next_slot,barrier,visible,recorded=(c[k] for k in ['launch','next_slot','barrier','visible','recorded'])
    pid=ui.pid();cfg=c['configs']['A'];store=root/'linked-store-A'
    # Same persisted source/speaker key exists in both private live meetings.
    folder=store/'new-live';folder.mkdir()
    (folder/'notes.md').write_text('Private Z notes')
    (folder/'transcript.json').write_text(json.dumps({'segments':[{'id':1,'start_ms':0,'end_ms':1000,'text':'Private Z persisted transcript.','source':'unknown','chunk_id':0}],'total_chunks':1}))
    with sqlite3.connect(store/'index.db') as db:db.execute('INSERT INTO meetings(id,title,started_at,duration_secs,status,chunk_count,storage_path,audio_retained) VALUES(?,?,?,?,?,?,?,?)',(z,'Stored Z',int(time.time()),1,'recording',1,str(folder),0))
    saved=ui.saved['A'][-1]
    (saved/'notes.md').write_text('Private Y saved notes')
    (saved/'asks.jsonl').write_text(json.dumps({'id':'private-saved-question','time':datetime.datetime.now(datetime.timezone.utc).isoformat(),'question':'Private saved Y question','answer':'Private saved Y answer','agent':'claude','agent_name':'Synthetic saved record','citations':[]})+'\n')
    def reconnect(new=False):
        if new:snapshot.update(meeting_id=z,title='Private genuinely new Z',segments=[{'id':1,'start_ms':0,'end_ms':1000,'text':'Private genuinely new Z transcript.','source':'unknown','chunk_id':0}])
        count=len(slots);slots[-1]['socket'].shutdown(c['socket'].SHUT_RDWR);slots[-1]['socket'].close();slot=next_slot(count);barrier(slot)
        if new:ui.wait(lambda:visible('Private genuinely new Z transcript.'),'New actual live Z must take over')
    def row(title):
        node=ui.wait(lambda:next((n for n in ui.inside('Meetings - Voxtype') if n.getRoleName()=='list item' and any(title in child.name for child in ui.walk(n))),None),'Missing actual manual meeting row '+title)
        point(node,activate=True)
        ui.wait(lambda:visible(title) and not visible('Recent meetings'),'Manual row must activate actual meeting view')
    def point(node,activate=False):
        windows=subprocess.check_output(['xdotool','search','--onlyvisible','--pid',str(pid),'--name','^Meetings - Voxtype$'],env=env,text=True).splitlines();assert len(windows)==1
        subprocess.run(['xdotool','windowfocus',windows[0]],env=env,check=True,timeout=5)
        label=next(n for n in ui.walk(node) if n.getRoleName()=='label')
        ext=label.queryComponent().getExtents(pyatspi.DESKTOP_COORDS)
        print('ACTUAL_ROW_INPUT_BOUNDS',json.dumps({'role':label.getRoleName(),'label':label.name,'bounds':[ext.x,ext.y,ext.width,ext.height],'activate':activate}),flush=True)
        # This GTK4 AT-SPI reports list-label positions as (0,0), and does not
        # implement row Action/grabFocus. Use real private XTest input at the
        # first-row coordinates inspected in the retained219 screenshots.
        geometry=subprocess.check_output(['xdotool','getwindowgeometry','--shell',windows[0]],env=env,text=True)
        dimensions=dict(line.split('=',1) for line in geometry.splitlines());assert (dimensions['WIDTH'],dimensions['HEIGHT'])==('980','680'),geometry
        assert label.name.startswith('● Recording  Private active X' if activate else '[00:00] Unknown: Private live X transcript.')
        subprocess.run(['xdotool','mousemove','--window',windows[0],'150','145' if activate else '63','click',*(['--repeat','2','--delay','100'] if activate else []),'1'],env=env,check=True,timeout=5)
    def barrier_now():
        marker='Private post-worker barrier '+str(time.monotonic_ns())
        owner=ui.id if case=='lifetime-worker-same' else snapshot['meeting_id']
        c['send'](slots[-1]['socket'],[{'event':'error','meeting_id':owner,'message':marker}])
        ui.wait(lambda:visible(marker),'Real view loop must cross worker completion barrier')
    def enter(frame,text):
        node=ui.wait(lambda:next((n for n in ui.inside(frame) if n.getRoleName() in ['text','entry'] and n.getState().contains(pyatspi.STATE_EDITABLE) and n.getState().contains(pyatspi.STATE_SHOWING) and n.queryText().getText(0,-1)==''),None),'Actual empty question/dialog entry missing')
        ext=node.queryComponent().getExtents(pyatspi.DESKTOP_COORDS)
        if frame=='Meetings - Voxtype':
            subprocess.run(['xdotool','mousemove',str(ext.x+ext.width//2),str(ext.y+ext.height//2),'click','1'],env=env,check=True,timeout=5)
        assert node.queryEditableText().setTextContents(text)
        return node
    def ask(q):
        enter('Meetings - Voxtype',q)
        subprocess.run(['xdotool','key','--clearmodifiers','Return'],env=env,check=True,timeout=5)
    def worker(key,meeting,q):
        record=ui.wait(lambda:json.loads((root/('worker-'+key+'.json')).read_text()) if (root/('worker-'+key+'.json')).exists() else None,'Real canonical Ask '+key+' must execute')
        assert record['argv']==['--config',str(cfg),'meeting','ask',meeting,q,'--json'],record
        assert record['rc']!=0 and b'private-unsupported-' in base64.b64decode(record['stdout']),record
        def owner():
            for task in Path('/proc',str(pid),'task').iterdir():
                try:
                    if str(record['pid']) in (task/'children').read_text().split():return task
                except FileNotFoundError:pass
        task=ui.wait(owner,'Actual GTK worker owning canonical observer must exist')
        return record,task
    def release(key,task):
        (root/('release-'+key)).touch()
        ui.wait(lambda:not task.exists(),'Actual GTK worker thread must finish after real stdout/exit release')
        barrier_now()
    if 'saved' in case:
        count=len(slots);launch('A',meeting=ui.id,ask_id='private-saved-question');slot=next_slot(count);barrier(slot)
        ui.wait(lambda:visible('Private saved Y answer'),'Requested saved answer must actually render')
        reconnect();assert visible('Private saved Y answer'),'Same-ID reconnect must preserve valid Y answer'
        if case.endswith('manual'):
            ui.press('Meetings - Voxtype','Meetings');row('Private active X')
        else:reconnect(True)
        assert not visible('Private saved Y question') and not visible('Private saved Y answer'),'Changed effective selection must detach saved Y answer'
    elif 'worker' in case:
        count=len(slots);launch('A',meeting=ui.id,tab='ask');slot=next_slot(count);barrier(slot)
        ask('Private held Y question');old,old_task=worker('Y',ui.id,'Private held Y question')
        if case.endswith('manual'):
            ui.press('Meetings - Voxtype','Meetings');row('Private active X');current=x
        elif case.endswith('same'):
            reconnect();current=ui.id
        else:reconnect(True);current=z
        old_presentation_leaked=not case.endswith('same') and visible('Private held Y question')
        cfg.write_text(cfg.read_text().replace('private-unsupported-A','private-unsupported-Z'))
        ask('Private held Z question');new,new_task=worker('Z',current,'Private held Z question')
        release('Y',old_task)
        print('ACTUAL_OLD_WORKER_RELEASED_CURRENT_UI',json.dumps({'case':case,'labels':[node.name for node in ui.inside('Meetings - Voxtype')],'old_worker':old,'current_worker':new}),flush=True)
        assert visible('Private held Z question') and visible('is answering') and not visible('private-unsupported-A'),'Old real error/exit must not alter or finish current Z answer'
        release('Z',new_task)
        ui.wait(lambda:visible('private-unsupported-Z'),'Current actual Z worker error must render in Z')
        assert not visible('private-unsupported-A') and not visible('is answering')
        assert not old_presentation_leaked,'Changed selection must also clear the old question presentation'
    else:
        action=case.split('-')[1];same=case.endswith('same')
        count=len(slots);launch('A');slot=next_slot(count);barrier(slot)
        if action=='rename':
            ui.press('Meetings - Voxtype','● Unknown (1)');dialog='Rename Unknown'
        elif action in ['move','note']:
            line=ui.wait(lambda:next((n for n in ui.inside('Meetings - Voxtype') if n.getRoleName()=='list item'),None),'Actual live transcript row missing')
            point(line)
            ui.press('Meetings - Voxtype','Move line to…' if action=='move' else 'Note on line…');dialog='Move this line to' if action=='move' else 'Note on this line'
        else:ui.press('Meetings - Voxtype','Stop');dialog='Stop the meeting?'
        ui.wait(lambda:ui.frame(dialog),'Real meeting-scoped dialog must open')
        baseline={str(p.relative_to(store)):p.read_bytes() for p in store.rglob('*') if p.is_file() and p.name!='index.db' and not p.name.startswith('index.db-')}
        c['lifetime_baseline']={name:base64.b64encode(data).decode() for name,data in baseline.items()}
        reconnect(not same)
        if action=='stop':ui.press(dialog,'Stop meeting')
        else:enter(dialog,'Private stale intent');ui.press(dialog,'OK')
        if same:
            ui.wait(lambda:any('label' in a and a[a.index('label')+1]==x for a in recorded()),'Same-ID rename must retain original canonical authority')
            ui.wait(lambda:visible('Rename: done'),'Actual same-ID rename must finish')
        else:
            ui.wait(lambda:visible('Meeting selection changed'),'Stale meeting dialog must be visibly rejected')
            assert not any(any(v in a for v in ['label','reassign','annotate','stop']) for a in recorded()),recorded()
            assert {str(p.relative_to(store)):p.read_bytes() for p in store.rglob('*') if p.is_file() and p.name!='index.db' and not p.name.startswith('index.db-')}==baseline,'Stale intent must not mutate any private meeting record'
    result={'case':case,'passed':True,'actual_argv':recorded(),'selected_title':snapshot['title'],'real_socket_connections':len(slots),'boundary':'private GTK, real canonical CLI/output, no audio/agent/network inference'}
    Path('/work/out/meeting-lifetime-'+case+'.json').write_text(json.dumps(result,indent=2));print('ACTUAL_SELECTED_MEETING_LIFETIME',json.dumps(result),flush=True)
    return result

def diagnostics(c):
    root=c['root'];store=root/'linked-store-A'
    records={p.name:json.loads(p.read_text()) for p in root.glob('worker-*.json')}
    files={str(p.relative_to(store)):base64.b64encode(p.read_bytes()).decode() for p in store.rglob('*') if p.is_file() and p.name!='index.db' and not p.name.startswith('index.db-')}
    print('ACTUAL_LIFETIME_PRIVATE_DIAGNOSTICS',json.dumps({'case':c['case'],'real_workers':records,'baseline':c.get('lifetime_baseline'),'after':files}),flush=True)
    for key in ['Y','Z']:(root/('release-'+key)).touch()
