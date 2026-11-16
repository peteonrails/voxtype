"""Real GTK accessibility controls and authoritative private CLI readback.
Run under private xvfb-run + dbus-run-session in the approved NAS container.
"""
import datetime
import json
import hashlib
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time

assert os.environ.get('HOME')=='/work/home'
assert os.environ.get('DISPLAY') and os.environ.get('DBUS_SESSION_BUS_ADDRESS')
assert not os.environ.get('WAYLAND_DISPLAY')
os.umask(0o077)
cli_launch = os.environ.get('VOX_HISTORY_GTK_LAUNCH') == 'cli'
case = os.environ.get('VOX_HISTORY_GTK_CASE','controls')
assert case in ['controls','search','details','correction','delete','copy','paste'], 'Only implemented, asserted journeys may run'
cli = os.environ.get('VOXTYPE_HISTORY_TEST_BIN','/work/target/debug/voxtype')
gtk = os.environ.get('VOXTYPE_HISTORY_TEST_GTK_BIN','/work/target/debug/voxtype-meetings-gtk4')
import pyatspi
from gi.repository import GLib
capture=Path(os.environ.get('VOX_HISTORY_GTK_CAPTURE','/work/out/history-gtk-controls.png'))
assert capture.parent==Path('/work/out') and not capture.exists(), 'Each journey requires a fresh owned screenshot path'

with tempfile.TemporaryDirectory(dir='/work/scratch',prefix='history-gtk-controls-') as name:
    root=Path(name)
    trace=os.environ.get('VOX_HISTORY_GTK_TRACE')=='1'
    if trace:
        # Observe the actual CLI requests/results without changing product
        # bytes or writing into the immutable artifact directory.
        binaries=root/'bin'; binaries.mkdir(mode=0o700)
        for original,destination in [(Path(cli),binaries/'voxtype-real'),(Path(gtk),binaries/'voxtype-meetings-gtk4')]:
            shutil.copyfile(original,destination); destination.chmod(0o500)
            assert hashlib.sha256(original.read_bytes()).digest()==hashlib.sha256(destination.read_bytes()).digest()
        (binaries/'voxtype').write_text('#!/usr/bin/python3\nimport json,os,subprocess,sys,time\nfrom pathlib import Path\nroot=Path(__file__).parent.parent\nwith (root/"backend-trace.jsonl").open("a") as log: log.write(json.dumps({"phase":"start","time":time.monotonic(),"argv":sys.argv[1:]})+"\\n")\nresult=subprocess.run([str(root/"bin/voxtype-real"),*sys.argv[1:]],stdin=subprocess.DEVNULL,capture_output=True)\nwith (root/"backend-trace.jsonl").open("a") as log: log.write(json.dumps({"phase":"end","time":time.monotonic(),"argv":sys.argv[1:],"rc":result.returncode,"stdout":result.stdout.decode(errors="replace"),"stderr":result.stderr.decode(errors="replace")})+"\\n")\nsys.stdout.buffer.write(result.stdout); sys.stderr.buffer.write(result.stderr); raise SystemExit(result.returncode)\n')
        (binaries/'voxtype').chmod(0o500)
        cli=str(binaries/'voxtype-real'); gtk=str(binaries/'voxtype-meetings-gtk4')
    now=datetime.datetime.now().astimezone()
    folder=root/'data/voxtype/history'
    folder.mkdir(parents=True,mode=0o700)
    entry_id=now.strftime('%Y%m%d-%H%M%S')+'-fedcba'
    entry: dict[str,object]=dict(schema=1,id=entry_id,ts_start=now.isoformat(),ts_end=now.isoformat(),raw_text='um synthetic invoice',text='Synthetic invoice.',engine='whisper',model='synthetic-model.gguf',delivered=False,outcome='output_failed',app='synthetic-editor')
    if case == 'details':
        entry['edits']=[{'from':'um','to':''}]
    (folder/(now.strftime('%Y-%m-%d')+'.jsonl')).write_text(json.dumps(entry)+'\n')
    config=root/'config.toml'
    config.write_text('[hotkey]\nenabled=false\n[history]\nenabled=true\n')
    runtime=root/'runtime'
    runtime.mkdir(mode=0o700)
    env=dict(os.environ,HOME=str(root),XDG_CONFIG_HOME=str(root/'config'),XDG_DATA_HOME=str(root/'data'),XDG_RUNTIME_DIR=str(runtime),GDK_BACKEND='x11',GSK_RENDERER='cairo',GTK_A11Y='atspi',VOXTYPE_MEETINGS_GTK4_SMOKE='15',VOXTYPE_MEETINGS_GTK4_SHOT=str(capture))
    sink=root/'clipboard-capture'
    if case in ['copy','paste']:
        tools=root/'tools'
        tools.mkdir(mode=0o700)
        helper=tools/'wl-copy'
        helper.write_text('#!/usr/bin/python3\nimport os,sys\nfrom pathlib import Path\nPath(os.environ["VOX_HISTORY_CAPTURE"]).write_bytes(sys.stdin.buffer.read())\n')
        helper.chmod(0o700)
        shutil.copyfile(helper,tools/'xclip'); (tools/'xclip').chmod(0o700)
        # The actual GUI is X11/Xvfb. Claiming a Wayland GUI here can launch
        # the standalone Qt surface alongside GTK and target the wrong window.
        env.update(PATH=str(tools)+':'+env['PATH'],XDG_SESSION_TYPE='x11',VOX_HISTORY_CAPTURE=str(sink))
        if case == 'paste':
            config.write_text(config.read_text()+'[output]\nmode="clipboard"\nappend_text=""\nwait_for_modifier_release=false\n')
            env['VOXTYPE_MEETINGS_GTK4_SHOT']=str(capture.with_name(capture.stem+'-hidden.png'))
    if case == 'copy':
        # Preserve the original wl-copy/logging JSON-stdout regression as well
        # as the X11 GTK button journey below. This command never opens a GUI.
        copy_env=dict(env,WAYLAND_DISPLAY='history-fixture-no-compositor',XDG_SESSION_TYPE='wayland')
        copied=subprocess.run([cli,'--config',str(config),'history','copy',entry_id,'--json'],env=copy_env,capture_output=True,text=True,timeout=10)
        assert copied.returncode==0,copied.stderr
        try:
            copied_value=json.loads(copied.stdout)
        except ValueError:
            raise AssertionError('history copy --json must reserve stdout for exactly JSON: '+repr(copied.stdout)) from None
        assert copied_value['copied']==entry_id
        assert sink.read_text()=='Synthetic invoice.'
        sink.unlink()  # The subsequent real button must produce its own output.
    args = [cli,'--config',str(config),'history','view'] if cli_launch else [gtk,'--history','--config',str(config)]
    gui=subprocess.Popen(args,env=env,stdin=subprocess.DEVNULL,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
    if cli_launch:
        launch_stdout,launch_stderr=gui.communicate(timeout=10)
        print(launch_stdout)
        print(launch_stderr)
        assert gui.returncode==0, 'History view must open the available graphical history window without a terminal'
    def walk(node,depth=0):
        if depth>30:
            return
        try:
            node.name
            count=node.childCount
        except GLib.Error:
            return  # AT-SPI can retain a proxy after its application hides/exits.
        yield node
        for i in range(count):
            try:
                child=node.getChildAtIndex(i)
            except GLib.Error:
                continue
            if child is not None:
                yield from walk(child,depth+1)
    def all_nodes():
        return list(walk(pyatspi.Registry.getDesktop(0)))
    def frame_visible(name):
        for node in all_nodes():
            try:
                if node.name==name and node.getState().contains(pyatspi.STATE_SHOWING):
                    return True
            except GLib.Error:
                continue
        return False
    def wait(check, timeout=10):
        deadline=time.monotonic()+timeout
        while True:
            while GLib.MainContext.default().pending():
                GLib.MainContext.default().iteration(False)
            value=check()
            if value:
                return value
            assert gui.poll() is None or (cli_launch and gui.returncode==0), 'GTK exited before the control journey'
            assert time.monotonic()<deadline, 'GTK control missing or not updated: '+repr([(n.name,n.getRoleName()) for n in all_nodes()])
            time.sleep(0.05)
    def activate(name):
        # GTK's Expander exposes its descendant text in the accessible name.
        # Keep all existing collection controls exact; identify only the real
        # Details expander by its native label prefix, then assert visible content.
        button=wait(lambda:next((n for n in all_nodes() if (n.name==name or (name=='Details' and n.name.startswith('Details '))) and n.getRoleName() in ['push button','button','toggle button'] and n.getState().contains(pyatspi.STATE_SHOWING)),None))
        assert button.queryAction().doAction(0), name
    def state():
        result=subprocess.run([cli,'--config',str(config),'history','status','--json'],env=env,capture_output=True,text=True,timeout=10)
        assert result.returncode==0,result.stderr
        return json.loads(result.stdout)
    def current_entry():
        result=subprocess.run([cli,'--config',str(config),'history','show',entry_id,'--json'],env=env,capture_output=True,text=True,timeout=10)
        assert result.returncode==0,result.stderr
        return json.loads(result.stdout)
    try:
        wait(lambda:any(n.getRoleName()=='application' and n.name=='voxtype-meetings-gtk4' for n in all_nodes()))
        print(json.dumps({'actual_accessible_applications':[n.name for n in all_nodes() if n.getRoleName()=='application']}))
        wait(lambda:any(n.name=='History on' for n in all_nodes()))
        if case == 'paste':
            activate('Paste again')
            activate('Cancel replay')
            wait(lambda:not any(n.name=='Paste dictation' and n.getState().contains(pyatspi.STATE_SHOWING) for n in all_nodes()))
            assert not sink.exists(), 'Cancel must not send any output'
            activate('Paste again')
            activate('Paste dictation')
            wait(lambda:not frame_visible('Voxtype History'))
            wait(lambda:sink.is_file())
            assert sink.read_text()=='Synthetic invoice.'
            assert state()['count']==1, 'Replay must not record another history entry'
            reopen_env=dict(env,VOXTYPE_MEETINGS_GTK4_SHOT=str(capture))
            reopened=subprocess.run([cli,'--config',str(config),'history','view'],env=reopen_env,capture_output=True,text=True,timeout=10)
            assert reopened.returncode==0,reopened.stderr
            wait(lambda:frame_visible('Voxtype History'))
        if case == 'copy':
            activate('Copy')
            wait(lambda:sink.is_file())
            assert sink.read_text()=='Synthetic invoice.'
            wait(lambda:any(n.name=='Copied transcript.' and n.getState().contains(pyatspi.STATE_SHOWING) for n in all_nodes()))
            assert current_entry()['id']==entry_id
        if case == 'delete':
            activate('Delete')
            activate('Cancel deletion')
            wait(lambda:not any(n.name=='Delete dictation' and n.getState().contains(pyatspi.STATE_SHOWING) for n in all_nodes()))
            assert current_entry()['id']==entry_id
            activate('Delete')
            activate('Delete dictation')
            wait(lambda:state()['count']==0)
            wait(lambda:any(n.name=='No dictations stored' for n in all_nodes()))
        if case == 'correction':
            def editor():
                return wait(lambda:next((n for n in all_nodes() if n.getRoleName()=='text' and n.getState().contains(pyatspi.STATE_EDITABLE) and n.getState().contains(pyatspi.STATE_SHOWING)),None))
            activate('Correct')
            assert editor().queryEditableText().setTextContents('Cancelled correction')
            activate('Cancel correction')
            wait(lambda:not any(n.name=='Correct dictation' and n.getState().contains(pyatspi.STATE_SHOWING) for n in all_nodes()))
            assert current_entry().get('corrected_text') is None
            activate('Correct')
            assert editor().queryEditableText().setTextContents('- Corrected invoice.\nSecond line.')
            activate('Save correction')
            wait(lambda:current_entry().get('corrected_text')=='- Corrected invoice.\nSecond line.')
            wait(lambda:any('[corrected]' in n.name for n in all_nodes()))
            assert current_entry()['raw_text']=='um synthetic invoice' and current_entry()['text']=='Synthetic invoice.'
            activate('Correct')
            assert editor().queryEditableText().setTextContents('')
            activate('Save correction')
            wait(lambda:current_entry().get('corrected_text')=='')
        if case == 'details':
            def shown(text):
                return any(n.name==text and n.getState().contains(pyatspi.STATE_SHOWING) for n in all_nodes())
            activate('Details')
            wait(lambda:shown('Raw:') and shown('um synthetic invoice'))
            wait(lambda:shown('Output:') and shown('Automatic edits:') and shown('- um\n+ '))
            activate('Details')
            wait(lambda:not shown('Raw:'))
            activate('Details')
            wait(lambda:shown('Raw:'))
        if case == 'search':
            search = wait(lambda:next((n for n in all_nodes() if n.getRoleName() in ['entry','text'] and n.getState().contains(pyatspi.STATE_EDITABLE)),None))
            assert search.queryEditableText().setTextContents('nonexistentneedle')
            wait(lambda:any(n.name=='No matching dictations or meetings' for n in all_nodes()))
            assert not any(n.name=='Synthetic invoice.' for n in all_nodes())
            assert search.queryEditableText().setTextContents('invoice')
            wait(lambda:any(n.name=='Synthetic invoice.' for n in all_nodes()))
            assert search.queryEditableText().setTextContents('')
            wait(lambda:any(n.name=='synthetic-editor | synthetic-model.gguf' for n in all_nodes()))
        for control,enabled,paused,label in [('Pause history',True,True,'History paused'),('Resume history',True,False,'History on'),('Disable history',False,False,'History off'),('Enable history',True,False,'History on')]:
            activate(control)
            wait(lambda:state()['enabled']==enabled and state()['paused']==paused)
            wait(lambda:any(n.name==label for n in all_nodes()))
            assert state()['count']==(0 if case=='delete' else 1), 'Collection controls must not delete prior entries'
        stdout,stderr=gui.communicate(timeout=25)
        print(stdout)
        print(stderr)
        assert gui.returncode==0
        if cli_launch or case=='paste':
            wait(lambda:capture.is_file(), timeout=20)
        assert capture.is_file()
        print(json.dumps({'case':'gtk-real-'+case+'-cli-launch' if cli_launch else 'gtk-real-'+case,'state':state(),'boundary':'actual GTK AT-SPI actions, private Xvfb/D-Bus, real CLI; not physical desktop acceptance'}))
    finally:
        if trace: print('PRIVATE_GUI_BEFORE_CLEANUP',gui.poll())
        if gui.poll() is None:
            gui.terminate()
            try:
                gui.wait(timeout=5)
            except subprocess.TimeoutExpired:
                gui.kill()
                gui.wait(timeout=5)
        if trace:
            print('PRIVATE_GUI_RETURN_CODE',gui.returncode)
            print('PRIVATE_BACKEND_TRACE', (root/'backend-trace.jsonl').read_text() if (root/'backend-trace.jsonl').exists() else '(no backend requests)')
            stdout,stderr=gui.communicate(timeout=5)
            print('PRIVATE_GUI_STDOUT',stdout)
            print('PRIVATE_GUI_STDERR',stderr)
