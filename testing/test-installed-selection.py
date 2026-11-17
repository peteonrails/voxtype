#!/usr/bin/env python3
"""Only run in the disposable namespace created by run-installed-selection.py."""
import importlib.util, json, os, pathlib, shutil, signal, subprocess, time
P=pathlib.Path
assert os.geteuid()==0 and P('/mnt/fixture').is_dir() and os.environ['DBUS_SESSION_BUS_ADDRESS'].endswith('absent-bus')
spec=importlib.util.spec_from_file_location('installer','/mnt/code/install-local.py');m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
e=P('/evidence');e.mkdir(exist_ok=True)
# Real output of the separately exercised normal-user bwrap repair planner.
# Nested uid-0 user namespaces cannot establish a second uid map on this host.
proposed=P('/mnt/prepared.toml').read_bytes();(e/'proposed.toml').write_bytes(proposed)
assert b'# old' in proposed and b'enabled = false' in proposed
# Real protected fixed-target stage/commit/verify, not a copy-layout approximation.
m.TARGET.parent.mkdir(parents=True);m.WRAPPER.parent.mkdir(parents=True,exist_ok=True)
m.WRAPPER.write_text('#!/bin/sh\nexec '+str(m.TARGET)+' "$@"\n');m.WRAPPER.chmod(0o755)
old=P('/usr/bin/sleep').read_bytes()
for target in m.DESTINATIONS:target.write_bytes(old);target.chmod(0o755)
wrapper=m.digest(m.WRAPPER.read_bytes());token='c'*32;follower=subprocess.Popen([str(m.TARGET),'90']);daemon=None
try:
 follower_start=m.process_start(follower.pid)
 m.root_stage(P('/mnt/code'),token);m.root_commit(token);m.root_verify(token,True)
 subprocess.run(['/usr/bin/python3','/mnt/code/reproduce/test-selection-inference.py','--bin',str(m.WRAPPER),'--models','/models','--fixtures','/mnt/fixtures','--evidence',str(e/'native')],check=True,timeout=120)
 native=json.loads((e/'native/results.json').read_text());assert len(native)==4
 for case in native:
  workers=case['worker_processes'].values()
  assert any(item['path']==str(m.TARGET.parent/'voxtype-gguf-worker-onnx-migraphx') and item['sha256']==m.PAYLOAD['voxtype-gguf-worker-vulkan'] for item in workers),case
 # Null ALSA device only: this namespace has no microphone, sound device, bus or production Wayland socket.
 alsa=e/'alsa-null.conf';alsa.write_text('pcm.!default { type null }\nctl.!default { type hw card 0 }\n')
 config=e/'daemon.toml';config.write_bytes(proposed+b'\n[hotkey]\nenabled=false\n[osd]\nenabled=false\n[audio.feedback]\nenabled=false\n')
 env=dict(os.environ,ALSA_CONFIG_PATH=str(alsa),XDG_DATA_HOME='/home/test/.local/share')
 stdout=(e/'daemon.stdout').open('w');stderr=(e/'daemon.stderr').open('w')
 daemon=subprocess.Popen([str(m.WRAPPER),'--config',str(config),'daemon'],env=env,stdout=stdout,stderr=stderr,start_new_session=True)
 runtime=P(os.environ['XDG_RUNTIME_DIR'])/'voxtype'
 def sample():
  if daemon.poll() is not None:return {'pid':0,'active':'failed'}
  lock=runtime/'voxtype.lock';status=runtime/'state'
  return {'pid':daemon.pid,'start':m.process_start(daemon.pid),'hash':m.digest(P('/proc',str(daemon.pid),'exe').read_bytes()),'restarts':0,'active':'active','sub':'running','result':'success','status':status.read_text().strip(),'control_pid':int(lock.read_text().strip())}
 health=m.stable_health(m.PAYLOAD['voxtype-vulkan'],sample=sample,duration=10,timeout=25,interval=.5)
 (e/'daemon-health.json').write_text(json.dumps({'scope':'real private null-audio daemon PID/start/hash/idle/lock for 10s; unit-state fields supplied, not real systemd','health':health},indent=2)+'\n')
 os.killpg(daemon.pid,signal.SIGTERM);daemon.wait(timeout=8);stdout.close();stderr.close();daemon=None
 assert follower.poll() is None and m.process_start(follower.pid)==follower_start
 m.root_restore(token)
 assert all(m.digest(m.read_regular(target,True))==m.digest(old) for target in m.DESTINATIONS)
 assert m.digest(m.WRAPPER.read_bytes())==wrapper and follower.poll() is None
 (e/'result.json').write_text(json.dumps({'passed':True,'selection_stage':True,'protected_exact_payload_apply_restore':True,'packaged_gpu_cases':len(native),'private_daemon_stable_seconds':10,'wrapper_preserved':True,'old_inode_follower_survived':True,'production_actions':False},indent=2)+'\n')
 print('INSTALLED_SELECTION_PASS: exact protected deployment, repair staging, four GPU transcriptions, private stable daemon, restore and follower survival')
finally:
 if daemon is not None:
  try:os.killpg(daemon.pid,signal.SIGTERM);daemon.wait(timeout=3)
  except (ProcessLookupError,subprocess.TimeoutExpired):
   try:os.killpg(daemon.pid,signal.SIGKILL);daemon.wait()
   except ProcessLookupError:pass
 follower.terminate();follower.wait()
