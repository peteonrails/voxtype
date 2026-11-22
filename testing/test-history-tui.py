"""NAS-only synthetic terminal smoke; no desktop/display/device access."""
import datetime
import codecs
import fcntl
import hashlib
import json
import os
from pathlib import Path
import pty
import re
import select
import shutil
import signal
import sqlite3
import struct
import subprocess
import tempfile
import termios
import time
import pyte
from PIL import Image, ImageDraw, ImageFont

assert Path('/work/out').is_dir()
os.umask(0o077)
exe = Path(os.environ.get('VOXTYPE_HISTORY_TEST_BIN','/work/target/debug/voxtype'))
assert exe.is_file()
raw = bytearray()
screen = pyte.Screen(150,40)
terminal = pyte.Stream(screen)
decoder = codecs.getincrementaldecoder('utf-8')()
case = os.environ.get('VOX_HISTORY_SMOKE_CASE','reader')
copy_cases=['copy','copy-raw-fallback','copy-corrected','copy-corrected-empty']
assert case in ['reader','search','correct','correct-leading-dash','delete','controls','paste','meeting-nav',*copy_cases]

def snapshot(label):
    # Drain the remaining frame after a matched cell. A match can arrive before
    # the footer's last write; do not mistake that partial frame for clipping.
    deadline=time.monotonic()+2
    while time.monotonic()<deadline and select.select([master],[],[],0.15)[0]:
        chunk=os.read(master,65536)
        raw.extend(chunk)
        terminal.feed(decoder.decode(chunk))
    text='\n'.join(screen.display)+'\n'
    Path('/work/out/history-view-'+label+'.txt').write_text(text)
    image=Image.new('RGB',(150*9,40*18),'#101218')
    draw=ImageDraw.Draw(image)
    try: font=ImageFont.truetype('/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf',14)
    except OSError: font=ImageFont.load_default()
    palette={'default':'#e4e4e7','yellow':'#e6d26a','cyan':'#73c8cc','white':'#e4e4e7','black':'#101218'}
    for y,line in enumerate(screen.display):
        for x,char in enumerate(line):
            cell=screen.buffer[y][x]
            color=palette.get(cell.fg,'#e4e4e7')
            if len(cell.fg)==6 and all(c in '0123456789abcdefABCDEF' for c in cell.fg): color='#'+cell.fg
            draw.text((x*9,y*18),char,font=font,fill=color)
    image.save('/work/out/history-view-'+label+'.png')
with tempfile.TemporaryDirectory(prefix='history-view-', dir='/work/scratch') as name:
    root = Path(name)
    data = root/'data/voxtype/history'
    data.mkdir(parents=True, mode=0o700)
    now = datetime.datetime.now().astimezone()
    entry_id = now.strftime('%Y%m%d-%H%M%S')+'-aabbcc'
    entry: dict[str, object] = dict(schema=1, id=entry_id, ts_start=now.isoformat(), ts_end=now.isoformat(), raw_text='um synthetic invoice', text='Synthetic invoice.', model='synthetic-model.gguf', app='synthetic-editor', delivered=False, outcome='output_failed')
    entry['edits'] = [{'from':'um', 'to':''}]
    if case=='copy-raw-fallback': entry['text']=''
    if case=='copy-corrected': entry['corrected_text']='- Corrected invoice.\nSecond line.'
    if case=='copy-corrected-empty': entry['corrected_text']=''
    (data/(now.date().isoformat()+'.jsonl')).write_text(json.dumps(entry)+'\n')
    config = root/'config.toml'
    config.write_text('[history]\nenabled = true\n')
    if case=='paste':config.write_text(config.read_text()+'[output]\nmode="clipboard"\nappend_text=""\nwait_for_modifier_release=false\n')
    env = {'HOME':str(root), 'XDG_DATA_HOME':str(root/'data'), 'XDG_CONFIG_HOME':str(root/'config'), 'XDG_RUNTIME_DIR':str(root/'runtime'), 'PATH':'', 'TERM':'xterm-256color', 'LC_ALL':'C.UTF-8'}
    clipboard_capture=root/'clipboard-capture'
    if case in [*copy_cases,'paste']:
        tools=root/'tools'; tools.mkdir(mode=0o700)
        helper=tools/'wl-copy'
        helper.write_text('''#!/usr/bin/python3
import os,sys,json,termios
from pathlib import Path
p=Path(os.environ["VOX_HISTORY_CAPTURE"])
data=sys.stdin.buffer.read()
p.write_bytes(data)
record={"argv":sys.argv,"uid":os.getuid(),"bytes":len(data),"capture_uid":p.stat().st_uid,"capture_mode":oct(p.stat().st_mode&0o777),"PATH":os.environ.get("PATH"),"WAYLAND_DISPLAY":os.environ.get("WAYLAND_DISPLAY")}
if "VOX_HISTORY_PRIVATE_PTY" in os.environ:
    fd=os.open(os.environ["VOX_HISTORY_PRIVATE_PTY"],os.O_RDONLY|os.O_NOCTTY)
    flags=termios.tcgetattr(fd)[3];os.close(fd)
    record["terminal_restored"]=bool(flags&termios.ICANON and flags&termios.ECHO)
with Path(os.environ["VOX_HISTORY_HELPER_LOG"]).open("a") as f:f.write(json.dumps(record)+"\\n")
print("PRIVATE_CLIPBOARD_HELPER "+json.dumps(record),file=sys.stderr)
''')
        helper.chmod(0o700)
        # The real clipboard availability probe executes `which wl-copy`.
        # Keep the capture-only PATH, but supply that actual trusted utility.
        trusted_which=shutil.which('which'); assert trusted_which
        (tools/'which').symlink_to(Path(trusted_which).resolve())
        env.update(PATH=str(tools),WAYLAND_DISPLAY='history-private-capture-only',XDG_SESSION_TYPE='wayland',VOX_HISTORY_CAPTURE=str(clipboard_capture),VOX_HISTORY_HELPER_LOG=str(root/'helper-log.jsonl'))
        probe=subprocess.run(['which','wl-copy'],env=env,capture_output=True,text=True,timeout=5)
        assert probe.returncode==0 and probe.stdout.strip()==str(helper),(probe.stdout,probe.stderr)
    args = [str(exe),'--config',str(config),'history']

    if case=='meeting-nav':
        assert os.environ.get('DISPLAY') and os.environ.get('DBUS_SESSION_BUS_ADDRESS'),'Controller must start private Xvfb/D-Bus'
        env.update(DISPLAY=os.environ['DISPLAY'],DBUS_SESSION_BUS_ADDRESS=os.environ['DBUS_SESSION_BUS_ADDRESS'],GDK_BACKEND='x11',GSK_RENDERER='cairo',GTK_A11Y='atspi')
        authority=Path(os.environ['XAUTHORITY'])
        assert authority.resolve().is_relative_to('/work/scratch') and authority.stat().st_uid==os.getuid()
        env['XAUTHORITY']=str(authority)
        tools=root/'tools';tools.mkdir(mode=0o700)
        (tools/'voxtype').symlink_to(exe)
        trusted_which=shutil.which('which');assert trusted_which
        (tools/'which').symlink_to(Path(trusted_which).resolve())
        # Meeting frontend resolution asks real ldd about the paired GTK
        # executable. Retain the immutable sandbox tool PATH, not the host's.
        env['PATH']=str(tools)+os.pathsep+os.environ['PATH']
        expected_gtk=Path(os.environ.get('VOXTYPE_HISTORY_TEST_GTK_BIN',str(exe.with_name('voxtype-meetings-gtk4'))))
        loader=subprocess.run(['ldd',str(expected_gtk)],env=env,capture_output=True,text=True,timeout=10)
        print('ACTUAL_MEETING_LOADER',loader.returncode,loader.stdout,loader.stderr,flush=True)
        assert loader.returncode==0 and '=> not found' not in loader.stdout,'Paired GTK dependency precondition must hold'
        meetings=root/'private-meetings'
        config.write_text(config.read_text()+'[meeting]\nview="gtk4"\nstorage_path='+json.dumps(str(meetings))+'\n')
        bootstrap=subprocess.run([str(exe),'--config',str(config),'meeting','list','--json'],env=env,capture_output=True,text=True,timeout=10)
        assert bootstrap.returncode==0,bootstrap.stderr
        meeting_id='11111111-2222-4333-8444-555555555555'
        saved_meeting=meetings/'synthetic-saved';saved_meeting.mkdir()
        transcript={'segments':[{'id':1,'start_ms':0,'end_ms':1000,'text':'Synthetic saved navigation transcript.','source':'unknown','chunk_id':0}],'total_chunks':1}
        (saved_meeting/'transcript.json').write_text(json.dumps(transcript))
        with sqlite3.connect(meetings/'index.db') as db:
            db.execute('INSERT INTO meetings(id,title,started_at,ended_at,duration_secs,status,chunk_count,storage_path,audio_retained) VALUES(?,?,?,?,?,?,?,?,?)',(meeting_id,'Synthetic navigationneedle meeting',int(now.timestamp()),int(now.timestamp()),1,'completed',1,str(saved_meeting),0))
        hits=subprocess.run(args+['search','navigationneedle','--json'],env=env,capture_output=True,text=True,timeout=10)
        assert hits.returncode==0,hits.stderr
        found=json.loads(hits.stdout)
        assert len(found)==1 and found[0]['source']=='meeting' and found[0]['meeting_id']==meeting_id,found
        print('REAL_MIXED_SEARCH_PRECONDITION',hits.stdout,flush=True)

    seeded = subprocess.run(args+['show',entry_id,'--json'],env=env,capture_output=True)
    assert seeded.returncode == 0, seeded.stderr
    assert json.loads(seeded.stdout)['id'] == entry_id
    if case=='meeting-nav' and os.environ.get('VOX_HISTORY_MEETING_DIAGNOSTIC')=='startup':
        import ctypes
        # Observe real orphaned canonical helper exits, not a replacement launcher.
        assert ctypes.CDLL(None,use_errno=True).prctl(36,1,0,0,0)==0
        from gi.repository import Gio,GLib
        bus=Gio.bus_get_sync(Gio.BusType.SESSION,None)
        def owner():
            try:return bus.call_sync('org.freedesktop.DBus','/org/freedesktop/DBus','org.freedesktop.DBus','GetConnectionUnixProcessID',GLib.Variant('(s)',('io.voxtype.Meetings',)),GLib.VariantType.new('(u)'),Gio.DBusCallFlags.NONE,1000,None).unpack()[0]
            except GLib.Error:return None
        def identity(pid):
            p=Path('/proc')/str(pid)
            return {'pid':pid,'sid':os.getsid(pid),'pgid':os.getpgid(pid),'cmdline':p.joinpath('cmdline').read_bytes().decode().split('\0'),'status':p.joinpath('status').read_text()}
        results=[]
        sealed_env=dict(env);sealed_env.pop('XAUTHORITY')
        for label,current in [('sealed179',sealed_env),('private-xauthority-control',dict(env))]:
            authority=Path(os.environ['XAUTHORITY'])
            assert authority.is_relative_to('/work/scratch') and authority.stat().st_uid==os.getuid()
            for route in ['direct-paired-gtk','canonical-non-pty']:
                command=[str(expected_gtk),'--meeting',meeting_id,'--config',str(config)] if route=='direct-paired-gtk' else [str(exe),'--config',str(config),'meeting','view',meeting_id]
                child_probe=subprocess.Popen(command,env=current,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
                record={'environment':label,'route':route,'parent':identity(os.getpid()),'initial_child':identity(child_probe.pid),'authority_present':'XAUTHORITY' in current,'cwd':os.getcwd(),'command':command}
                try:
                    stdout,stderr=child_probe.communicate(timeout=2)
                    record.update(exit=child_probe.returncode,stdout=stdout.decode(),stderr=stderr.decode(),fixture_terminated=False)
                except subprocess.TimeoutExpired:
                    record.update(alive_after_two_seconds=True,observed_child=identity(child_probe.pid),dbus_owner=owner())
                    child_probe.terminate();stdout,stderr=child_probe.communicate(timeout=5)
                    record.update(exit=child_probe.returncode,stdout=stdout.decode(),stderr=stderr.decode(),fixture_terminated=True)
                if route=='canonical-non-pty':
                    deadline=time.monotonic()+2
                    while time.monotonic()<deadline:
                        try:pid,status=os.waitpid(-1,os.WNOHANG)
                        except ChildProcessError:break
                        if pid:
                            record.setdefault('actual_adopted_child_exits',[]).append({'pid':pid,'wait_status':status,'exit':os.waitstatus_to_exitcode(status)})
                            break
                        app_pid=owner()
                        if app_pid:
                            record['actual_helper']=identity(app_pid)
                            os.kill(app_pid,signal.SIGTERM)
                            pid,status=os.waitpid(app_pid,0)
                            record['fixture_terminated_helper']={'pid':pid,'wait_status':status,'exit':os.waitstatus_to_exitcode(status)}
                            break
                        time.sleep(.05)
                results.append(record)
                print('REAL_MEETING_STARTUP_COMPARISON',json.dumps(record),flush=True)
        Path('/work/out/meeting-startup-comparison.json').write_text(json.dumps(results,indent=2))
        raise SystemExit(0)
    master, slave = pty.openpty()
    if case=='paste':env['VOX_HISTORY_PRIVATE_PTY']=os.ttyname(slave)
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH',40,150,0,0))
    startup_observations=[]
    startup_stop=None
    if case=='meeting-nav' and os.environ.get('VOX_HISTORY_MEETING_LIFETIME_DIAGNOSTIC'):
        import ctypes,threading
        assert ctypes.CDLL(None,use_errno=True).prctl(36,1,0,0,0)==0
        startup_stop=threading.Event()
        def observe_helpers():
            previous={}
            while not startup_stop.is_set():
                for directory in Path('/proc').iterdir():
                    if not directory.name.isdigit():continue
                    try:
                        command=directory.joinpath('cmdline').read_bytes().split(b'\0')
                        if os.fsencode(config) not in command or not command or command[0] not in [os.fsencode(exe),os.fsencode(expected_gtk)]:continue
                        pid=int(directory.name)
                        observation={'pid':pid,'command':[os.fsdecode(a) for a in command],'stat':directory.joinpath('stat').read_text(),'status':directory.joinpath('status').read_text(),'stdio':[os.readlink(directory/'fd'/str(fd)) for fd in [0,1,2]]}
                        signature=(tuple(command),tuple(observation['stat'].split()[3:8]),tuple(line for line in observation['status'].splitlines() if line.startswith(('SigIgn:','SigCgt:'))),tuple(observation['stdio']))
                        if previous.get(pid)!=signature:
                            startup_observations.append(observation);previous[pid]=signature
                    except (FileNotFoundError,ProcessLookupError):continue
                startup_stop.wait(.001)
        startup_thread=threading.Thread(target=observe_helpers,daemon=True);startup_thread.start()
    def start_session():
        os.setsid()
        fcntl.ioctl(0, termios.TIOCSCTTY, 0)
        if os.environ.get('VOX_HISTORY_MEETING_LIFETIME_DIAGNOSTIC')=='ignore-hup':signal.signal(signal.SIGHUP,signal.SIG_IGN)
    child = subprocess.Popen(args+['view','--tui'],env=env,stdin=slave,stdout=slave,stderr=slave,preexec_fn=start_session)
    os.close(slave)
    def until(text):
        deadline = time.monotonic()+12
        while text.decode('utf-8') not in '\n'.join(screen.display):
            assert time.monotonic() < deadline, ('Timed out waiting for '+repr(text), bytes(raw))
            if select.select([master],[],[],0.1)[0]:
                try:
                    chunk = os.read(master,65536)
                except OSError:
                    raise AssertionError(('Terminal exited early',child.poll(),bytes(raw)))
                raw.extend(chunk)
                terminal.feed(decoder.decode(chunk))
                if b'\x1b[6n' in chunk:
                    os.write(master,b'\x1b[1;1R')
            assert child.poll() is None, ('Terminal exited early',child.returncode,bytes(raw))
    try:
        expected_text=entry.get('corrected_text',entry['text'] or entry['raw_text'])
        if expected_text: until(str(expected_text).split('\n')[0].encode())
        until(b'synthetic-editor')
        until(b'synthetic-model.gguf')
        until(b'[failed]')
        snapshot('list')
        os.write(master,b'\r')
        until(b'um synthetic invoice')
        until(b'Automatic edits:')
        snapshot('details')
        if case in copy_cases:
            os.write(master,b'c')
            until(b'Copied transcript.' if expected_text else b'This transcript is empty.')
            if expected_text: assert clipboard_capture.read_bytes()==str(expected_text).encode()
            else: assert not clipboard_capture.exists(),'Intentional empty correction must not recover raw text or invoke the clipboard helper'
            cli_copy=subprocess.run(args+['copy',entry_id,'--json'],env=env,capture_output=True,text=True,timeout=10)
            assert cli_copy.returncode==0,cli_copy.stderr
            assert json.loads(cli_copy.stdout)['copied']==entry_id,'CLI stdout must be one parseable JSON result'
            if expected_text:
                assert clipboard_capture.read_bytes()==str(expected_text).encode()
                events=[json.loads(line) for line in (root/'helper-log.jsonl').read_text().splitlines()]
                assert len(events)==2 and all(event['uid']==65532 and event['capture_uid']==65532 and event['capture_mode']=='0o600' and event['argv'][1:]==['--type','text/plain;charset=utf-8'] for event in events),events
            else: assert not clipboard_capture.exists() and not (root/'helper-log.jsonl').exists()
            unchanged=subprocess.run(args+['show',entry_id,'--json'],env=env,capture_output=True,text=True,timeout=10)
            assert unchanged.returncode==0,unchanged.stderr
            saved=json.loads(unchanged.stdout)
            assert saved['raw_text']==entry['raw_text'] and saved['text']==entry['text'] and saved.get('corrected_text')==entry.get('corrected_text') and saved['outcome']=='output_failed'
            snapshot(case)
            print(json.dumps({'copy_case':case,'expected_bytes':str(expected_text).encode().hex(),'cli_stdout':cli_copy.stdout,'capture_only':True}),flush=True)
        if case == 'controls':
            for key,enabled,paused,status in [(b'p',True,True,b'History: paused'),(b'p',True,False,b'History: on'),(b'h',False,False,b'History: off'),(b'h',True,False,b'History: on')]:
                os.write(master,key)
                until(status)
                state=subprocess.run(args+['status','--json'],env=env,capture_output=True,text=True,timeout=10)
                assert state.returncode==0,state.stderr
                value=json.loads(state.stdout)
                assert value['enabled']==enabled and value['paused']==paused and value['count']==1,value
            snapshot('collection-controls')
        if os.environ.get('VOX_HISTORY_SMOKE_CASE') == 'delete':
            os.write(master,b'd')
            until(b'Delete this dictation?')
            os.write(master,b'n')
            still_saved=subprocess.run(args+['show',entry_id,'--json'],env=env,capture_output=True)
            assert still_saved.returncode == 0, still_saved.stderr
            os.write(master,b'd')
            until(b'Delete this dictation?')
            os.write(master,b'y')
            until(b'No dictations stored')
            listed=subprocess.run(args+['list','--json'],env=env,capture_output=True)
            assert listed.returncode == 0 and json.loads(listed.stdout)['entries']==[], listed.stderr
            snapshot('deleted')
        if case in ['correct','correct-leading-dash']:
            correction = '- synthetic invoice --json literal' if case == 'correct-leading-dash' else 'Corrected invoice.'
            os.write(master,b'e')
            until(b'Correct transcript:')
            os.write(master,b'\x15'+correction.encode()+b'\r')
            until(b'[corrected]')
            until(correction.encode())
            corrected=subprocess.run(args+['show',entry_id,'--json'],env=env,capture_output=True)
            assert corrected.returncode == 0, corrected.stderr
            saved=json.loads(corrected.stdout)
            assert saved['raw_text']=='um synthetic invoice'
            assert saved['text']=='Synthetic invoice.'
            assert saved['corrected_text']==correction
            snapshot('corrected')
        if os.environ.get('VOX_HISTORY_SMOKE_CASE') == 'search':
            os.write(master,b'/')
            until(b'Search history and meetings:')
            snapshot('search')
            os.write(master,b'nonexistent-private-fixture-term\r')
            until(b'No matches')
            snapshot('no-matches')
        if case=='meeting-nav':
            os.write(master,b'/')
            until(b'Search history and meetings:')
            os.write(master,b'navigationneedle\r')
            until(b'Meeting: Synthetic navigationneedle meeting')
            snapshot('meeting-result')
            os.write(master,b'o')
            rc=child.wait(timeout=12)
            if startup_stop is not None:
                time.sleep(.2)
                startup_stop.set();startup_thread.join(timeout=2)
                exited=[]
                while True:
                    try:pid,status=os.waitpid(-1,os.WNOHANG)
                    except ChildProcessError:break
                    if not pid:break
                    exited.append({'pid':pid,'wait_status':status,'exit':os.waitstatus_to_exitcode(status)})
                print('ACTUAL_PTY_HELPER_LIFETIME',json.dumps({'signal_control':os.environ.get('VOX_HISTORY_MEETING_LIFETIME_DIAGNOSTIC'),'observations':startup_observations,'adopted_child_exits':exited}),flush=True)
            while select.select([master],[],[],0)[0]:
                try:chunk=os.read(master,65536)
                except OSError:break
                if not chunk:break
                raw.extend(chunk)
            print('CANONICAL_MEETING_VIEW_EXIT',rc,'canonical_mode',bool(termios.tcgetattr(master)[3]&termios.ICANON),'terminal_tail',bytes(raw[-6000:]),flush=True)
            assert rc==0 and termios.tcgetattr(master)[3]&termios.ICANON,'Meeting navigation must leave raw mode first'
            import pyatspi
            from gi.repository import Gio,GLib
            connection=Gio.bus_get_sync(Gio.BusType.SESSION,None)
            def walk(node,depth=0):
                if depth>25:return
                try:count=node.childCount;node.name
                except GLib.Error:return
                yield node
                for i in range(count):
                    try:member=node.getChildAtIndex(i)
                    except GLib.Error:continue
                    if member is not None:yield from walk(member,depth+1)
            def visible(text):
                for node in walk(pyatspi.Registry.getDesktop(0)):
                    try:
                        if text in node.name and node.getState().contains(pyatspi.STATE_SHOWING):return True
                    except GLib.Error:pass
                return False
            deadline=time.monotonic()+12
            while not (visible('Synthetic navigationneedle meeting') and visible('Synthetic saved navigation transcript.')):
                if time.monotonic()>=deadline:
                    actual=[]
                    for node in walk(pyatspi.Registry.getDesktop(0)):
                        try:
                            if node.getState().contains(pyatspi.STATE_SHOWING):actual.append({'role':node.getRoleName(),'name':node.name})
                        except GLib.Error:pass
                    print('ACTUAL_MEETING_WINDOW_ACCESSIBILITY',json.dumps(actual),flush=True)
                    show=subprocess.run([str(exe),'--config',str(config),'meeting','show',meeting_id,'--json'],env=env,capture_output=True,text=True,timeout=10)
                    print('ACTUAL_SAVED_MEETING_SHOW',show.returncode,show.stdout,show.stderr,flush=True)
                    try:
                        pid=connection.call_sync('org.freedesktop.DBus','/org/freedesktop/DBus','org.freedesktop.DBus','GetConnectionUnixProcessID',GLib.Variant('(s)',('io.voxtype.Meetings',)),GLib.VariantType.new('(u)'),Gio.DBusCallFlags.NONE,1000,None).unpack()[0]
                        print('ACTUAL_MEETING_OWNER',pid,(Path('/proc')/str(pid)/'cmdline').read_bytes(),flush=True)
                    except GLib.Error as error:print('ACTUAL_MEETING_OWNER_ERROR',str(error),flush=True)
                assert time.monotonic()<deadline,'Actual canonical GTK meeting window must render the selected saved transcript, not just an IPC request'
                time.sleep(.05)
            observed=[]
            for node in walk(pyatspi.Registry.getDesktop(0)):
                try:
                    if node.getState().contains(pyatspi.STATE_SHOWING):observed.append({'role':node.getRoleName(),'name':node.name})
                except GLib.Error:pass
            Path('/work/out/history-meeting-navigation-accessible.json').write_text(json.dumps(observed,indent=2))
            from PIL import ImageGrab
            assert Path(os.environ['XAUTHORITY']).resolve().is_relative_to('/work/scratch')
            ImageGrab.grab(xdisplay=os.environ['DISPLAY']).save('/work/out/history-meeting-navigation-window.png')
            owner=connection.call_sync('org.freedesktop.DBus','/org/freedesktop/DBus','org.freedesktop.DBus','GetConnectionUnixProcessID',GLib.Variant('(s)',('io.voxtype.Meetings',)),GLib.VariantType.new('(u)'),Gio.DBusCallFlags.NONE,1000,None).unpack()[0]
            descriptor=os.pidfd_open(owner)
            try:
                expected_gtk=Path(os.environ.get('VOXTYPE_HISTORY_TEST_GTK_BIN',str(exe.with_name('voxtype-meetings-gtk4'))))
                assert os.fsencode(expected_gtk) in (Path('/proc')/str(owner)/'cmdline').read_bytes().split(b'\0')
                assert os.getpgid(owner)==owner and os.getpgid(owner)!=child.pid,'Canonical GTK helper must outlive the exited controlling-session reader in its own process group'
                if os.environ.get('VOX_HISTORY_MEETING_LIFETIME_DIAGNOSTIC')!='ignore-hup':
                    ignored=next(line.split(':',1)[1].strip() for line in (Path('/proc')/str(owner)/'status').read_text().splitlines() if line.startswith('SigIgn:'))
                    assert not int(ignored,16)&1,'Actual regression must pass without ignoring SIGHUP'
                print(json.dumps({'case':'actual-TUI-to-canonical-GTK-saved-meeting','meeting_id':meeting_id,'gtk_pid':owner,'title_and_saved_transcript_visible':True,'terminal_restored':True}),flush=True)
                signal.pidfd_send_signal(descriptor,signal.SIGTERM)
            finally:os.close(descriptor)
        elif case=='paste':
            os.write(master,b'a')
            until(b'Send this transcript again?')
            os.write(master,b'n')
            until(b'c copy')
            assert not clipboard_capture.exists(),'Cancelling replay must not deliver text'
            os.write(master,b'a')
            until(b'Send this transcript again?')
            snapshot('paste-confirm')
            os.write(master,b'y')
            rc=child.wait(timeout=12)
            assert clipboard_capture.read_bytes()==b'Synthetic invoice.'
            events=[json.loads(line) for line in (root/'helper-log.jsonl').read_text().splitlines()]
            assert len(events)==1 and events[0]['terminal_restored'] and events[0]['uid']==65532,events
            assert termios.tcgetattr(master)[3]&termios.ICANON
            saved=subprocess.run(args+['show',entry_id,'--json'],env=env,capture_output=True,text=True,timeout=10)
            assert saved.returncode==0 and json.loads(saved.stdout)['outcome']=='output_failed'
            print(json.dumps({'case':'TUI-confirmed-replay','events':events,'cancelled_without_delivery':True,'original_outcome_unchanged':True}),flush=True)
        else:
            os.write(master,b'q')
            rc = child.wait(timeout=12)
        assert rc == 0, (rc,bytes(raw))
    finally:
        if startup_stop is not None:
            startup_stop.set();startup_thread.join(timeout=2)
        if child.poll() is None:
            child.kill()
            child.wait()
        os.close(master)
        Path('/work/out/history-view-terminal.ansi').write_bytes(raw)
    Path('/work/out/history-view-terminal-result.json').write_text(json.dumps({'case':case,'terminal_validation':'pyte-decoded current terminal grid with retained grid text and headless raster captures; not a physical desktop acceptance','classification':'bounded development evidence, not final certification','child_rc':rc,'synthetic_fixture':True,'validated':['CLI reads same seeded entry','terminal recent list','app and model metadata','failed marker','expanded raw text','automatic edits','clean exit'],'executable_sha256':hashlib.sha256(exe.read_bytes()).hexdigest(),'terminal_capture_sha256':hashlib.sha256(raw).hexdigest()},indent=2))
