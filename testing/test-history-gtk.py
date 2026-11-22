"""Private headless GTK history frontdoor acceptance; NAS only."""
import datetime
import json
import os
from pathlib import Path
import subprocess
import tempfile

assert os.environ.get('HOME') == '/work/home'
assert not os.environ.get('DISPLAY') and not os.environ.get('WAYLAND_DISPLAY')
os.umask(0o077)
with tempfile.TemporaryDirectory(dir='/work/scratch',prefix='history-gtk-') as name:
    root=Path(name)
    now=datetime.datetime.now().astimezone()
    folder=root/'data/voxtype/history'
    folder.mkdir(parents=True,mode=0o700)
    entry_id=now.strftime('%Y%m%d-%H%M%S')+'-fedcba'
    entry=dict(schema=1,id=entry_id,ts_start=now.isoformat(),ts_end=now.isoformat(),raw_text='um synthetic invoice',text='Synthetic invoice.',engine='whisper',model='synthetic-model.gguf',delivered=False,outcome='output_failed',app='synthetic-editor')
    (folder/(now.strftime('%Y-%m-%d')+'.jsonl')).write_text(json.dumps(entry)+'\n')
    config=root/'config.toml'
    config.write_text('[hotkey]\nenabled=false\n[history]\nenabled=true\n')
    runtime=root/'runtime'
    runtime.mkdir(mode=0o700)
    env=dict(os.environ,HOME=str(root),XDG_CONFIG_HOME=str(root/'config'),XDG_DATA_HOME=str(root/'data'),XDG_RUNTIME_DIR=str(runtime),GDK_BACKEND='x11',GSK_RENDERER='cairo',VOXTYPE_MEETINGS_GTK4_SMOKE='2',VOXTYPE_MEETINGS_GTK4_SHOT='/work/out/history-gtk-list.png')
    env.pop('DBUS_SESSION_BUS_ADDRESS',None)
    result=subprocess.run(['xvfb-run','-a','-s','-screen 0 1280x900x24 -nolisten tcp','dbus-run-session','--','/work/target/debug/voxtype-meetings-gtk4','--history','--config',str(config)],env=env,capture_output=True,text=True,timeout=25)
    print(result.stdout)
    print(result.stderr)
    assert result.returncode==0, 'GTK history frontdoor must accept --history: '+result.stderr
    report=json.loads(next(line for line in result.stdout.splitlines() if line.startswith('{')))
    assert report['screen']=='history', report
    assert report['state']=='History on', report
    assert report['entries']==1, report
    assert report['ids']==[entry_id], report
    assert Path('/work/out/history-gtk-list.png').is_file()
    print(json.dumps({'case':'gtk-real-history-list','report':report,'boundary':'private Xvfb with synthetic history, not production display or physical desktop acceptance'}))
