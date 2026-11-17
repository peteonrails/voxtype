#!/usr/bin/env python3
"""Test only the unprivileged repair planner; candidate runs in its real bwrap.
All HOME/state/config/model-copy paths are disposable. No service/sudo call.
"""
import argparse, importlib.util, json, os, pathlib, shutil, tempfile
p=argparse.ArgumentParser(description=__doc__);p.add_argument('--installer',required=True,type=pathlib.Path);p.add_argument('--bundle',required=True,type=pathlib.Path);p.add_argument('--model',required=True,type=pathlib.Path);p.add_argument('--evidence',required=True,type=pathlib.Path);a=p.parse_args();e=a.evidence.resolve();e.mkdir(parents=True,exist_ok=True)
assert os.geteuid()!=0,'This is the normal-user planner test, not a root apply'
spec=importlib.util.spec_from_file_location('installer',a.installer.resolve());assert spec and spec.loader;m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
results=[]
with tempfile.TemporaryDirectory(dir=e) as temporary:
 home=pathlib.Path(temporary);os.environ['HOME']=str(home)
 model=home/'.local/share/voxtype/models/parakeet-tdt-0.6b-v3-Q8_0.gguf';model.parent.mkdir(parents=True);shutil.copyfile(a.model,model)
 original=b'# preserve\nengine = "parakeet"\n[parakeet]\nmodel = "parakeet-tdt-0.6b-v2" # old\nmodel_type = "tdt"\nstreaming = false\non_demand_loading = false\n[cohere]\nmodel = "cohere-transcribe-03-2026-Q4_K_M.gguf"\nlanguage = "en"\n[tray]\nenabled = false\n'
 live=home/'live-config.toml';live.write_bytes(original);state=home/'state';state.mkdir()
 proposed=m.stage_selection(a.bundle.resolve(),state,original);assert live.read_bytes()==original
 assert b'# old' in proposed and b'enabled = false' in proposed and b'parakeet-tdt-0.6b-v3-Q8_0.gguf' in proposed
 (e/'proposed.toml').write_bytes(proposed);results.append('real pinned-candidate selection planning preserves original bytes')
 os.rename(model,model.with_suffix('.missing'))
 try:m.verify_selection_model();raise AssertionError('missing model accepted')
 except FileNotFoundError:results.append('missing model refused')
 os.rename(model.with_suffix('.missing'),model)
 with model.open('r+b') as stream:
  old=stream.read(1);stream.seek(0);stream.write(b'X')
 try:m.verify_selection_model();raise AssertionError('wrong model digest accepted')
 except RuntimeError:results.append('wrong model hash refused')
 with model.open('r+b') as stream:stream.write(old)
 sample={'MainPID':'0','ActiveState':'activating','SubState':'auto-restart','Result':'exit-code','NRestarts':'7'}
 for changes,success in [({},True),({'MainPID':'123'},False),({'ActiveState':'active','SubState':'running'},False),({'Result':'success'},False)]:
  fixture=dict(sample,**changes);m.unit_state=lambda:fixture
  try:
   m.failed_selection_service();assert success
  except RuntimeError:assert not success
  results.append('failed-service gate '+json.dumps(changes))
 try:m.stage_selection(a.bundle.resolve(),home,original.replace(b'engine = "parakeet"',b'engine = "cohere"'));raise AssertionError('scope change accepted')
 except RuntimeError:results.append('other engine repair refused')
 try:m.stage_selection(a.bundle.resolve(),home,None);raise AssertionError('absent original config accepted')
 except RuntimeError as error:
  assert 'existing config' in str(error);results.append('absent original config refused explicitly')
 # Orchestration fixture: root/service calls are spies; config restore uses real files.
 # Exact privileged root operations are exercised separately in namespace fixtures.
 backup=home/'rollback';backup.mkdir();(backup/'config.before').write_bytes(original)
 meta={'enable_tray':False,'repair_selection':True,'original_ready':False,'config_existed':True,'plugin_existed':False,'plugin_moved':False,'osd_existed':False,'osd_moved':False,'presets_added':[]}
 for fail_root in [False,True]:
  actions=[];live.write_bytes(proposed)
  def root_call(*args):
   actions.append('root restore')
   if fail_root:raise RuntimeError('fixture root restore failure')
  m.root_call=root_call;m.run=lambda *args:actions.append(' '.join(str(value) for value in args))
  m.restore_plugin=lambda *args:actions.append('plugin restore');m.osd_assets_path=lambda:home/'osd';m.restore_osd=lambda *args:actions.append('osd restore');m.restore_presets=lambda *args:actions.append('preset restore');m.rescan=lambda:None
  m.stable_health=lambda *args:(_ for _ in ()).throw(AssertionError('original broken service must not be called healthy'))
  try:m.recovery('fixture-code','a'*32,backup,home/'plugin',live,meta,'unused');assert not fail_root
  except RuntimeError:assert fail_root
  assert live.read_bytes()==original and any('start voxtype' in action for action in actions)
  assert 'plugin restore' in actions and 'osd restore' in actions and 'preset restore' in actions
  results.append('real config restoration and all recovery attempts; root/service spies; root failure='+str(fail_root))
 assert live.read_bytes()==original
 (e/'result.json').write_text(json.dumps({'passed':True,'count':len(results),'cases':results,'scope':'normal-user installer planner and candidate bwrap only; no sudo/service/config mutation on production','proposed_sha256':m.digest(proposed)},indent=2)+'\n')
print(json.dumps({'passed':len(results),'cases':results},indent=2))
