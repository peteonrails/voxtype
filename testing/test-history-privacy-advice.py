"""NAS-only: follow actual printed privacy advice against isolated configs."""
import hashlib,json,os,re,shutil,subprocess,tempfile,tomllib
from pathlib import Path
assert os.getuid()==65532 and Path('/work/out').is_dir()
assert not any(os.environ.get(k) for k in ['DISPLAY','WAYLAND_DISPLAY','SWAYSOCK','DBUS_SESSION_BUS_ADDRESS'])
os.umask(0o077)
exe=Path(os.environ['VOXTYPE_HISTORY_TEST_BIN']);digest=hashlib.sha256(exe.read_bytes()).hexdigest()
records=[];failures=[]
for context in ['explicit','default']:
 for state,action in [('off','enable'),('paused','resume'),('on','pause')]:
  with tempfile.TemporaryDirectory(prefix='privacy-advice-',dir='/work/scratch') as directory:
   root=Path(directory);default=root/'config/voxtype/config.toml';default.parent.mkdir(parents=True,mode=0o700)
   explicit=root/"chosen ' ; $(printf private-injection) [x].toml"
   chosen,sentinel=(explicit,default) if context=='explicit' else (default,explicit)
   text='# Chosen private fixture\n[history]\nenabled='+('false' if state=='off' else 'true')+'\npaused='+('true' if state=='paused' else 'false')+'\n'
   chosen.write_text(text);sentinel.write_text('# Unchosen sentinel\n[history]\nenabled=false\npaused=true\n');before=sentinel.read_bytes()
   tools=root/'tools';tools.mkdir();(tools/'voxtype').symlink_to(exe);which=shutil.which('which');assert which;(tools/'which').symlink_to(Path(which).resolve())
   env=dict(HOME=str(root),XDG_CONFIG_HOME=str(root/'config'),XDG_DATA_HOME=str(root/'data'),XDG_RUNTIME_DIR=str(root/'runtime'),TMPDIR=str(root),PATH=str(tools),LC_ALL='C.UTF-8',DBUS_SESSION_BUS_ADDRESS='unix:path='+str(root/'absent-bus'))
   argv=[str(exe),*(['--config',str(chosen)] if context=='explicit' else []),'setup','check']
   check=subprocess.run(argv,env=env,capture_output=True,text=True,timeout=15);assert check.returncode==0,check.stderr
   output=re.sub(r'\x1b\[[0-9;]*m','',check.stdout)
   lines=[line[line.index('voxtype'):].strip() for line in output.splitlines() if 'voxtype' in line and re.search(r'\bhistory '+action+r'\s*$',line)]
   record=dict(context=context,state=state,chosen=str(chosen),setup_argv=argv,setup_rc=check.returncode,setup_stdout=check.stdout,setup_stderr=check.stderr,printed_commands=lines,native_sha256=digest)
   try:
    assert len(lines)==1,('Expected one actual actionable advice line',lines)
    command=lines[0]
    actual=subprocess.run(['/bin/sh','-c',command],env=env,cwd=root,capture_output=True,text=True,timeout=15)
    record.update(followed_shell_command=command,follow_rc=actual.returncode,follow_stdout=actual.stdout,follow_stderr=actual.stderr,selected_after=tomllib.loads(chosen.read_text()),sentinel_unchanged=sentinel.read_bytes()==before)
    generations=list((root/'data/voxtype/history-control').glob('*/generation'));record['generations']={p.parent.name:int.from_bytes(p.read_bytes(),'little') for p in generations}
    assert actual.returncode==0,record
    h=record['selected_after']['history'];assert h['enabled']==(action=='enable' or state!='off') and h['paused']==(action=='pause'),('Printed command did not mutate selected context',record)
    assert sentinel.read_bytes()==before,('Printed command mutated default/unselected context',record)
    selected_key=hashlib.sha256(os.fsencode(chosen.resolve())).hexdigest()
    assert record['generations']==({selected_key:1} if action=='pause' else {}),record
    normalized=' '.join(output.split())
    assert 'loaded configuration' in normalized and 'live daemon' in normalized and 'CLI/environment overrides' in normalized,('State/override scope missing',output)
    assert 'legacy logging' in normalized and 'file output' in normalized,('Independent persistence caveat missing',output)
    assert set(root.iterdir()) <= {chosen,sentinel,tools,root/'config',root/'data',root/'runtime',root/'chosen.toml.tmp',root/'chosen.toml.bak',Path(str(chosen)+'.bak')},'Unexpected shell side effect'
    record['passed']=True
   except AssertionError as e:record.update(passed=False,failure=str(e));failures.append(dict(context=context,state=state,failure=str(e)))
   records.append(record)
result=dict(passed=not failures,native_sha256=digest,records=records,failures=failures,scope='Actual printed shell commands followed; no daemon/audio/output/inference. Two isolated files and privacy-generation authority checked.')
Path('/work/out/history-privacy-advice.json').write_text(json.dumps(result,indent=2)+'\n');print('ACTUAL_PRINTED_PRIVACY_COMMANDS',json.dumps(result),flush=True)
assert not failures,failures
