"""Real ConfigEditor CLI saves with original private modes under ordinary022."""
import hashlib,json,os,subprocess,tempfile,tomllib
from pathlib import Path
assert os.getuid()==65532 and Path('/work/out').is_dir()
assert not any(os.environ.get(k) for k in ['DISPLAY','WAYLAND_DISPLAY','DBUS_SESSION_BUS_ADDRESS'])
exe=Path(os.environ['VOXTYPE_HISTORY_TEST_BIN']);digest=hashlib.sha256(exe.read_bytes()).hexdigest();records=[];failures=[]
for context in ['explicit','default']:
 for kind in ['0600','0640','new','poison-tmp','poison-backup','existing-backup']:
  os.umask(0o077)
  with tempfile.TemporaryDirectory(prefix='private-save-',dir='/work/scratch') as directory:
   root=Path(directory);default=root/'config/voxtype/config.toml';default.parent.mkdir(parents=True,mode=0o700);explicit=root/'chosen.toml'
   chosen,sentinel=(explicit,default) if context=='explicit' else (default,explicit)
   text='# Private save fixture\n[history]\nenabled=true\npaused=false\nrecord_window_title=false # preserve title comment\n'
   if kind!='new':chosen.write_text(text);chosen.chmod(0o640 if kind=='0640' else 0o600)
   sentinel.write_text('# Unselected sentinel\n[history]\nenabled=false\n');sentinel.chmod(0o600);sentinel_before=sentinel.read_bytes()
   tmp=Path(str(chosen)+'.tmp');bak=Path(str(chosen)+'.bak');victim=root/'owned-negative-sentinel'
   assert not tmp.exists() and not tmp.is_symlink()
   if kind=='poison-tmp':victim.write_text('Owned sentinel, not config\n');tmp.symlink_to(victim)
   if kind=='poison-backup':bak.symlink_to(victim)
   if kind=='existing-backup':bak.write_text('Existing backup must survive\n')
   old_bak=bak.read_bytes() if kind=='existing-backup' else None
   env=dict(HOME=str(root),XDG_CONFIG_HOME=str(root/'config'),XDG_DATA_HOME=str(root/'data'),XDG_RUNTIME_DIR=str(root/'runtime'),PATH='',LC_ALL='C.UTF-8',TMPDIR=str(root),DBUS_SESSION_BUS_ADDRESS='unix:path='+str(root/'absent-bus'))
   args=[str(exe),*(['--config',str(chosen)] if context=='explicit' else [])]
   os.umask(0o022)
   before=chosen.read_bytes() if chosen.exists() else None;before_mode=chosen.stat().st_mode&0o777 if chosen.exists() else None
   bad=subprocess.run(args+['config','set','history.retention_days','--','-1'],env=env,capture_output=True,text=True,timeout=10)
   assert bad.returncode!=0 and (chosen.read_bytes() if chosen.exists() else None)==before and (chosen.stat().st_mode&0o777 if chosen.exists() else None)==before_mode,'Rejected edit must preserve bytes/mode'
   child=subprocess.run(args+['config','set','history.record_window_title','true'],env=env,capture_output=True,text=True,timeout=10)
   record=dict(context=context,kind=kind,argv=args+['config','set','history.record_window_title','true'],native_sha256=digest,rc=child.returncode,stdout=child.stdout,stderr=child.stderr,original_mode=oct(before_mode) if before_mode is not None else None,final_mode=oct(chosen.stat().st_mode&0o777) if chosen.exists() else None,rejected_edit_rc=bad.returncode,sentinel_unchanged=sentinel.read_bytes()==sentinel_before,stale_tmp_is_symlink=tmp.is_symlink(),backup_is_symlink=bak.is_symlink())
   try:
    assert child.returncode==0,record
    assert chosen.stat().st_mode&0o777==(0o640 if kind=='0640' else 0o600),('Config save widened permissions under022',record)
    assert sentinel.read_bytes()==sentinel_before,record
    assert tomllib.loads(chosen.read_text())['history']['record_window_title'] is True
    if kind!='new':assert '# Private save fixture' in chosen.read_text() and '# preserve title comment' in chosen.read_text()
    if kind=='poison-tmp':assert victim.read_text()=='Owned sentinel, not config\n' and tmp.is_symlink(),'Must not follow or rename a preexisting .tmp symlink'
    if kind=='poison-backup':assert not victim.exists() and bak.is_symlink(),'Must not follow a preexisting dangling .bak symlink'
    if kind=='existing-backup':assert bak.read_bytes()==old_bak
    if kind not in ['poison-backup','existing-backup','new']:assert bak.stat().st_mode&0o777==0o600,'New backup must start private'
    record['passed']=True
   except AssertionError as e:record.update(passed=False,failure=str(e));failures.append(dict(context=context,kind=kind,failure=str(e)))
   records.append(record)
os.umask(0o077)
result=dict(passed=not failures,native_sha256=digest,records=records,failures=failures,umask_during_native_save='022',scope='Actual CLI ConfigEditor and validation failure controls; all poison paths confined to owned disposable roots. TUI/cancel acceptance measured separately.')
Path('/work/out/history-private-save.json').write_text(json.dumps(result,indent=2)+'\n');print('ACTUAL_PRIVATE_CONFIG_SAVE',json.dumps(result),flush=True);assert not failures,failures
