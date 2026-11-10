#!/usr/bin/env python3
"""Non-root apply/rollback orchestration: real user files and candidate probes.
Sudo, privileged deployment and systemd are explicit spies. Exact protected-root
operations and the real GPU daemon are exercised separately in namespace tests.
No call can write a production destination or contact the daily service.
"""
import argparse,contextlib,importlib.util,io,json,os,pathlib,stat,subprocess,tempfile,types
p=argparse.ArgumentParser(description=__doc__);p.add_argument('--installer',required=True,type=pathlib.Path);p.add_argument('--bundle',required=True,type=pathlib.Path);p.add_argument('--models',required=True,type=pathlib.Path);p.add_argument('--prepared',required=True,type=pathlib.Path);p.add_argument('--evidence',required=True,type=pathlib.Path);a=p.parse_args();e=a.evidence.resolve();e.mkdir(parents=True,exist_ok=True)
assert os.geteuid()!=0,'Must exercise a normal-user-owned config, never root-owned ancestors'
original=b'# preserve\nengine = "parakeet"\n[parakeet]\nmodel = "parakeet-tdt-0.6b-v2" # old\nmodel_type = "tdt"\nstreaming = false\non_demand_loading = false\n[cohere]\nmodel = "cohere-transcribe-03-2026-Q4_K_M.gguf"\nlanguage = "en"\n[tray]\nenabled = false\n';proposed=a.prepared.read_bytes();results=[];real_subprocess_run=subprocess.run
for injected_failure in [False,True]:
 with tempfile.TemporaryDirectory(dir=e) as temporary:
  home=pathlib.Path(temporary);os.environ['HOME']=str(home);os.environ['XDG_DATA_HOME']=str(home/'.local/share');spec=importlib.util.spec_from_file_location('installer',a.installer.resolve());assert spec and spec.loader;m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
  config=home/'.config/voxtype/config.toml';config.parent.mkdir(parents=True);config.write_bytes(original);config.chmod(0o640)
  plugin=home/'.config/omarchy/plugins/io.voxtype.settings';plugin.mkdir(parents=True);(plugin/'old-marker').write_text('keep original plugin')
  target=home/'fixture-app';target.write_bytes(b'original application marker');wrapper=home/'fixture-wrapper';wrapper.write_bytes(b'wrapper fixture, must not execute')
  old_hash=m.digest(target.read_bytes());wrapper_hash=m.digest(wrapper.read_bytes());calls=[];saw_commit=[];m.TARGET=target;m.WRAPPER=wrapper
  raw_read=m.read_regular
  m.read_regular=lambda path,protected=False:raw_read(path,False if pathlib.Path(path) in [target,wrapper] else protected)
  m.check_wrapper=lambda:wrapper_hash
  m.preflight=lambda bundle,repair_selection=False:{'engine':'parakeet','service':{'pid':0},'repair_selection':True,'original_ready':False,'target_sha256':old_hash,'wrapper_sha256':wrapper_hash}
  m.failed_selection_service=lambda:{'pid':0}
  def stage(snapshot,state,before):
   assert before==original;calls.append('real planner output supplied');return proposed
  m.stage_selection=stage;m.verify_selection_model=lambda:'verified separately by real planner test'
  def root_call(code,action,token,source=None):
   calls.append('root-spy '+action)
   if action=='commit':target.write_bytes(b'new application marker')
   if action=='restore':target.write_bytes(b'original application marker')
   return {'records':[{'sha256':old_hash}]}
  m.root_call=root_call;m.rescan=lambda:None
  def authorized(args,*rest,**kwargs):
   if list(args)==['/usr/bin/sudo','-v']:
    calls.append('sudo-spy authentication');return subprocess.CompletedProcess(args,0,'','')
   return real_subprocess_run(args,*rest,**kwargs)
  m.subprocess.run=authorized
  def run(*args):
   if str(args[0])=='/usr/bin/systemctl':
    calls.append('systemd-spy '+' '.join(map(str,args[1:])));return subprocess.CompletedProcess(args,0,'','')
   assert pathlib.Path(args[0])==wrapper,args
   assert config.read_bytes()==proposed,'candidate validation must follow actual user config commit'
   saw_commit.append(True)
   if injected_failure:raise RuntimeError('fixture failure after user config commit')
   snapshot=next((home/'.local/state/voxtype-local-install').glob('*/snapshot'))
   command=['/usr/bin/bwrap','--unshare-all','--die-with-parent','--new-session','--clearenv','--ro-bind','/usr','/usr','--ro-bind','/etc','/etc','--symlink','usr/bin','/bin','--symlink','usr/lib','/lib','--symlink','usr/lib','/lib64','--proc','/proc','--dev','/dev','--tmpfs','/tmp','--tmpfs','/run','--ro-bind',str(snapshot),'/bundle','--ro-bind',str(a.models.resolve()),'/data/voxtype/models','--ro-bind',str(config.parent),'/fixture','--setenv','HOME','/tmp','--setenv','XDG_DATA_HOME','/data','--setenv','PATH','/usr/bin:/bin','/bundle/voxtype-vulkan','--config','/fixture/config.toml','config','validate-selection']
   return real_subprocess_run(command,check=True,capture_output=True,text=True,timeout=30)
  # Keep original probe implementation: its candidate/ldd commands really run in bwrap.
  original_run=m.run
  def dispatch(*args):
   if args and str(args[0])=='/usr/bin/bwrap':return original_run(*args)
   return run(*args)
  m.run=dispatch
  m.stable_health=lambda expected:{'fixture':True,'meaning':'service state is a spy; actual daemon tested in namespace'}
  output=io.StringIO()
  try:
   with contextlib.redirect_stdout(output):m.apply(a.bundle.resolve(),m.helper_code(),False,True)
   assert not injected_failure,'injected failure did not propagate'
   assert saw_commit and config.read_bytes()==proposed and stat.S_IMODE(config.stat().st_mode)==0o640
   token=next((home/'.local/state/voxtype-local-install').iterdir()).name
   with contextlib.redirect_stdout(output):m.rollback(m.helper_code(),token)
   assert config.read_bytes()==original and target.read_bytes()==b'original application marker'
   assert (plugin/'old-marker').read_text()=='keep original plugin'
   assert 'original non-ready selection restored' in output.getvalue()
   results.append({'case':'normal-user apply and rollback','passed':True,'config_mode':oct(stat.S_IMODE(config.stat().st_mode))})
  except RuntimeError as error:
   (e/('failure-'+str(injected_failure)+'.txt')).write_text(str(error)+'\n')
   assert config.read_bytes()==original,'failed apply did not restore original config'
   assert target.read_bytes()==b'original application marker'
   if not injected_failure:
    (e/'red.json').write_text(json.dumps({'expected_bug_observed':'Unprotected directory' in str(error),'error':str(error),'calls':calls},indent=2)+'\n')
    raise AssertionError('RED: real non-root apply cannot commit the user config: '+str(error)) from error
   assert saw_commit and 'fixture failure after user config commit' in str(error)
   assert any('systemd-spy --user start voxtype'==call for call in calls)
   results.append({'case':'failure after actual user config commit restores config/root/plugin and starts original','passed':True})
  finally:
   m.subprocess.run=real_subprocess_run;(e/('orchestration-'+str(injected_failure)+'.log')).write_text(output.getvalue());(e/('calls-'+str(injected_failure)+'.json')).write_text(json.dumps(calls,indent=2)+'\n')
(e/'result.json').write_text(json.dumps({'passed':True,'cases':results,'scope':'real non-root apply/config commit/rollback; root, sudo, systemd and health are explicit spies; real candidate probes use bwrap'},indent=2)+'\n');print(json.dumps(results,indent=2))
