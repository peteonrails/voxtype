"""Real history file replay through private synthetic files/hooks/clipboard sink."""
import datetime,json,os,shlex,shutil,subprocess,tempfile
from pathlib import Path
assert os.environ.get('HOME')=='/work/home'
assert not any(os.environ.get(k) for k in ['DISPLAY','WAYLAND_DISPLAY','DBUS_SESSION_BUS_ADDRESS','SWAYSOCK','NIRI_SOCKET','HYPRLAND_INSTANCE_SIGNATURE'])
os.umask(0o077)
exe=Path(os.environ.get('VOXTYPE_HISTORY_TEST_BIN','/work/target/debug/voxtype'))
with tempfile.TemporaryDirectory(dir='/work/scratch',prefix='history-file-replay-') as name:
 root=Path(name); tools=root/'bin'; tools.mkdir()
 for tool in ['sh','cat','which']:
  location=shutil.which(tool); assert location; (tools/tool).symlink_to(location)
 sink=root/'clipboard-capture'; hook_log=root/'hooks'; shell=shutil.which('sh'); assert shell
 (tools/'wl-copy').write_text('#!'+shell+'\ncat > '+shlex.quote(str(sink))+'\n'); (tools/'wl-copy').chmod(0o700)
 config=root/'config/voxtype/config.toml'; config.parent.mkdir(parents=True)
 folder=root/'data/voxtype/history'; folder.mkdir(parents=True,mode=0o700)
 runtime=root/'runtime'; runtime.mkdir(mode=0o700)
 now=datetime.datetime.now().astimezone(); entry_id=now.strftime('%Y%m%d-%H%M%S')+'-abc123'
 day=folder/(now.strftime('%Y-%m-%d')+'.jsonl')
 original=json.dumps(dict(schema=1,id=entry_id,ts_start=now.isoformat(),ts_end=now.isoformat(),raw_text='synthetic original',text='Synthetic original.',engine='whisper',model='synthetic.gguf',delivered=True,outcome='delivered'))+'\n'
 day.write_text(original)
 env=dict(HOME=str(root),XDG_CONFIG_HOME=str(root/'config'),XDG_DATA_HOME=str(root/'data'),XDG_RUNTIME_DIR=str(runtime),TMPDIR=str(root),PATH=str(tools),WAYLAND_DISPLAY='synthetic-no-compositor-socket')
 for mode in ['overwrite','append','fallback','failure']:
  if sink.exists(): sink.unlink()
  hook_log.write_text(''); target=root/'transcript.txt'
  target.write_text('previous\n')
  fallback=mode=='fallback'
  if mode in ['fallback','failure']:
   blocker=root/('blocked-'+mode); blocker.write_text('synthetic non-directory'); target=blocker/'transcript.txt'
  pre="printf 'pre\\n' >> "+shlex.quote(str(hook_log)); post="printf 'post\\n' >> "+shlex.quote(str(hook_log))
  config.write_text('[history]\nenabled=true\n[output]\nmode="file"\nfile_path='+json.dumps(str(target))+'\nfile_mode='+json.dumps(mode if mode in ['append','overwrite'] else 'overwrite')+'\nfallback_to_clipboard='+str(fallback).lower()+'\npre_output_command='+json.dumps(pre)+'\npost_output_command='+json.dumps(post)+'\n')
  result=subprocess.run([str(exe),'--config',str(config),'history','--json','paste',entry_id,'--delay-ms','0'],env=env,capture_output=True,text=True,timeout=15)
  payload=json.loads(result.stdout)
  if mode=='failure':
   assert result.returncode!=0,(mode,payload)
   assert not sink.exists(), 'disabled file fallback must not copy text'
  else:
   assert result.returncode==0,(mode,payload,result.stderr)
   if fallback:
    assert sink.read_text()=='Synthetic original.'
    assert payload['fell_back'] is True
   else:
    expected=('previous\n' if mode=='append' else '')+'Synthetic original.\n'
    assert target.read_text()==expected,(mode,'File-mode replay failed to write the configured file',payload)
    assert payload['driver']=='file',payload
    assert not sink.exists(), 'successful file replay must not copy to the clipboard'
  assert hook_log.read_text()=='pre\npost\n', (mode,hook_log.read_text())
  assert day.read_text()==original, 'replay must not rewrite or duplicate history'
  print(json.dumps({'case':mode,'producer_exit':result.returncode,'result':payload,'private_only':True}))
