"""Actual installer + canonical CLI + private-compositor history launch.
Only synthetic HOME/config/history; no production endpoints or audio.
"""
import datetime,json,os,shutil,stat,subprocess,tempfile,time
from pathlib import Path
assert os.environ.get('HOME')=='/work/home'
case=os.environ.get('VOX_HISTORY_LAUNCH_CASE','default')
assert case in ['default','forwarded-config','meeting-link']
assert not any(os.environ.get(k) for k in ['DISPLAY','WAYLAND_DISPLAY','SWAYSOCK','HYPRLAND_INSTANCE_SIGNATURE'])
assert bool(os.environ.get('DBUS_SESSION_BUS_ADDRESS')) == (case in ['forwarded-config','meeting-link']), 'Graphical action cases require their own controller-started private D-Bus session'
os.umask(0o077)
source=Path(__file__).resolve().parents[1]
exe=Path(os.environ.get('VOXTYPE_HISTORY_TEST_BIN','/work/target/debug/voxtype'))
with tempfile.TemporaryDirectory(dir='/work/scratch',prefix='history-launch-') as name:
 root=Path(name)
 runtime=root/'runtime'; runtime.mkdir(mode=0o700)
 config=root/'config/voxtype/config.toml'; config.parent.mkdir(parents=True)
 config.write_text('[hotkey]\nenabled=false\n[history]\nenabled=true\n')
 caller_a=root/'caller-a'; caller_b=root/'caller-b'
 caller_a.mkdir(); caller_b.mkdir()
 config_a=caller_a/'config.toml'; config_b=caller_b/'config.toml'
 config_a.write_text('[history]\nenabled=true\npaused=false\n')
 config_b.write_text('[history]\nenabled=false\npaused=false\n')
 original_a=config_a.read_bytes(); original_default=config.read_bytes()
 now=datetime.datetime.now().astimezone()
 folder=root/'data/voxtype/history'; folder.mkdir(parents=True,mode=0o700)
 entry_id=now.strftime('%Y%m%d-%H%M%S')+'-abc123'
 (folder/(now.strftime('%Y-%m-%d')+'.jsonl')).write_text(json.dumps(dict(schema=1,id=entry_id,ts_start=now.isoformat(),ts_end=now.isoformat(),raw_text='synthetic launch',text='Synthetic launch.',engine='whisper',model='synthetic.gguf',delivered=True,outcome='delivered'))+'\n')
 env=dict(os.environ,HOME=str(root),XDG_CONFIG_HOME=str(root/'config'),XDG_DATA_HOME=str(root/'data'),XDG_RUNTIME_DIR=str(runtime),WLR_BACKENDS='headless',WLR_RENDERER='pixman',WLR_LIBINPUT_NO_DEVICES='1',WLR_HEADLESS_OUTPUTS='1',QT_QPA_PLATFORM='wayland',QT_QUICK_BACKEND='software',QT_WAYLAND_DISABLE_WINDOWDECORATION='1',QML_DISABLE_DISK_CACHE='1',QS_DISABLE_FILE_WATCHER='1',QS_NO_RELOAD_POPUP='1',QT_LINUX_ACCESSIBILITY_ALWAYS_ON='1',QT_ACCESSIBILITY='1')
 links=None
 if case=='meeting-link':
  from history_meeting_links import MeetingLinks
  env.update(GDK_BACKEND='wayland',GSK_RENDERER='cairo',GTK_A11Y='atspi')
  links=MeetingLinks(root,env,exe,{'A':config_a,'B':config_b,'D':config})
 target=root/'data/voxtype/quickshell'
 installed=subprocess.run([str(exe),'setup','quickshell','--source',str(source/'quickshell'),'--target',str(target),'--skip-bridge'],env=env,capture_output=True,text=True,timeout=20)
 assert installed.returncode==0,installed.stdout+installed.stderr
 assert (target/'history.qml').is_file(), 'Actual setup quickshell must install the history application entrypoint'
 assert (target/'voxtype-meetings/HistoryWindow.qml').is_file(), 'Actual installer omitted the shared history window'
 assert (target/'voxtype-meetings/HistoryPanel.qml').is_file(), 'Actual installer omitted the shared history panel'
 env['VOXTYPE_OSD_QML_PATH']=str(target)
 env['VOXTYPE_HISTORY_BIN']=str(links.client_bin if links is not None else exe)
 if links is not None and links.failure_mode:
  # Preserve real qs/native payloads while observing the child-only CLI path.
  real_qs=shutil.which('qs',path=env['PATH']);assert real_qs
  observer=root/'qs-observer';observer.mkdir()
  qs=observer/'qs';qs.write_text('#!/usr/bin/python3\nimport os,sys\nos.environ["VOXTYPE_HISTORY_BIN"]='+repr(links.client_bin)+'\nos.execv('+repr(real_qs)+',['+repr(real_qs)+',*sys.argv[1:]])\n');qs.chmod(0o500)
  env['PATH']=str(observer)+os.pathsep+env['PATH']
 sway_config=root/'sway.conf'; sway_config.write_text('output HEADLESS-1 mode 1280x900\nseat * fallback true\nfocus_follows_mouse no\n')
 with (root/'sway.stdout').open('xb') as stdout,(root/'sway.stderr').open('xb') as stderr:
  compositor=subprocess.Popen(['sway','-c',str(sway_config)],env=env,stdin=subprocess.DEVNULL,stdout=stdout,stderr=stderr)
  try:
   deadline=time.monotonic()+15
   while True:
    sockets=[p for p in runtime.glob('wayland-*') if stat.S_ISSOCK(p.lstat().st_mode)]
    ipc=list(runtime.glob('sway-ipc*.sock'))
    if sockets and ipc: break
    assert compositor.poll() is None,(root/'sway.stderr').read_text()
    assert time.monotonic()<deadline,'Private compositor startup timed out'
    time.sleep(.05)
   env.update(WAYLAND_DISPLAY=sockets[0].name,SWAYSOCK=str(ipc[0]))
   def nodes(node):
    yield node
    for child in node.get('nodes',[])+node.get('floating_nodes',[]): yield from nodes(child)
   def windows():
    response=subprocess.run(['swaymsg','-t','get_tree','-r'],env=env,capture_output=True,text=True,timeout=5)
    assert response.returncode==0,response.stderr
    return [node for node in nodes(json.loads(response.stdout)) if node.get('name')=='VoxType History']
   def launch(argument=None,cwd=None):
    result=subprocess.run([str(exe),*(['--config',argument] if argument is not None else []),'history','view'],cwd=cwd,env=env,stdin=subprocess.DEVNULL,capture_output=True,text=True,timeout=15)
    print(result.stdout); print(result.stderr)
    assert result.returncode==0,'Canonical history frontdoor failed: '+result.stderr
    deadline=time.monotonic()+20
    while not windows():
     assert time.monotonic()<deadline,'Canonical launch did not map its actual staged Quickshell history window'
     time.sleep(.05)
   if case=='meeting-link':
    assert links is not None
    def linked_launch(label):
     launch(None if label=='D' else './config.toml',None if label=='D' else (caller_a if label=='A' else caller_b))
     actual=windows();assert len(actual)==1 and actual[0].get('shell')=='xdg_shell',actual
     assert os.fsencode(target/'history.qml') in (Path('/proc')/str(actual[0]['pid'])/'cmdline').read_bytes().split(b'\0')
    links.exercise(linked_launch,'standalone')
    raise SystemExit(0)
   launch('config.toml',caller_a) if case=='forwarded-config' else launch()
   initial=windows(); assert len(initial)==1 and initial[0].get('shell')=='xdg_shell',initial
   actual_arguments=(Path('/proc')/str(initial[0]['pid'])/'cmdline').read_bytes().split(b'\0')
   assert os.fsencode(target/'history.qml') in actual_arguments,'This gate must reach the actual installed standalone entry, not GTK or another fallback'
   launch('config.toml',caller_a) if case=='forwarded-config' else launch()
   time.sleep(.3)
   assert len(windows())==1,'Idempotent canonical launch opened duplicate windows'
   shot=subprocess.run(['grim','-o','HEADLESS-1','/work/out/history-staged-launch.png'],env=env,capture_output=True,text=True,timeout=5)
   assert shot.returncode==0,shot.stderr
   if case=='forwarded-config':
    import pyatspi
    from gi.repository import GLib
    def walk(node,depth=0):
     if depth>25:return
     try:count=node.childCount; node.name
     except GLib.Error:return
     yield node
     for i in range(count):
      try:child=node.getChildAtIndex(i)
      except GLib.Error:continue
      if child is not None:yield from walk(child,depth+1)
    def showing(node):
     try:return node.getState().contains(pyatspi.STATE_SHOWING)
     except GLib.Error:return False
    def visible(text):return next((n for n in walk(pyatspi.Registry.getDesktop(0)) if n.name==text and showing(n)),None)
    def wait(check,message):
     deadline=time.monotonic()+10
     while True:
      value=check()
      if value:return value
      assert compositor.poll() is None,'Private compositor exited'
      assert time.monotonic()<deadline,message
      time.sleep(.05)
    wait(lambda:visible('History on'),'Real standalone first-window accessibility/state precondition unavailable')
    launch('./config.toml',caller_b)
    wait(lambda:visible('History off'),'Existing standalone must forward caller B config instead of reusing A')
    button=wait(lambda:visible('Enable history'),'Actual standalone enable control missing')
    assert button.queryAction().doAction(0)
    def config_state(path):
     result=subprocess.run([str(exe),'--config',str(path),'history','--json','status'],env=env,capture_output=True,text=True,timeout=5)
     assert result.returncode==0,result.stderr
     return json.loads(result.stdout)
    wait(lambda:config_state(config_b)['enabled'] and visible('History on'),'Actual standalone enable must save and reload B')
    button=wait(lambda:visible('Pause history'),'Actual standalone pause control missing')
    assert button.queryAction().doAction(0)
    wait(lambda:config_state(config_b)['paused'] and visible('History paused'),'Actual standalone pause must save and reload B')
    assert config_a.read_bytes()==original_a and config.read_bytes()==original_default,'Standalone B controls must not write A/default'
    launch('config.toml',caller_a)
    wait(lambda:visible('History on') and not visible('History paused'),'Reopening A must restore its immutable client')
    assert len(windows())==1,'Only the current history window is visible'
    launch()
    wait(lambda:visible('History on'),'Absent config must select the default client')
    button=wait(lambda:visible('Pause history'),'Default client pause control missing')
    assert button.queryAction().doAction(0)
    wait(lambda:config_state(config)['paused'] and visible('History paused'),'Absent config must mutate default, not the original process config A')
    assert config_a.read_bytes()==original_a and config_state(config_b)['paused'],'Default client must not change A or B'
   print(json.dumps({'case':'actual-installed-quickshell-canonical-launch','path_case':case,'surface':windows()[0],'entry_id':entry_id,'boundary':'actual setup command and public CLI in private Sway; not production deployment'}))
  finally:
   compositor.terminate()
   try: compositor.wait(timeout=5)
   except subprocess.TimeoutExpired:
    compositor.kill(); compositor.wait(timeout=5)
