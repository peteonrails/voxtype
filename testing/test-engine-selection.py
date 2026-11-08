#!/usr/bin/env python3
"""Real CLI and PTY persistence regression. Use only in a private runtime/HOME.
No microphone, playback, daemon start, model download or normal TUI quit.
"""
import argparse, fcntl, json, os, pathlib, pty, select, signal, struct, subprocess, tempfile, termios, time, tomllib
p=argparse.ArgumentParser(description=__doc__)
p.add_argument('--bin',required=True,type=pathlib.Path)
p.add_argument('--models',required=True,type=pathlib.Path)
p.add_argument('--evidence',required=True,type=pathlib.Path)
p.add_argument('--tui',action='store_true')
a=p.parse_args(); binary=a.bin.resolve(); evidence=a.evidence.resolve(); evidence.mkdir(parents=True,exist_ok=True)
assert os.environ.get('DBUS_SESSION_BUS_ADDRESS','').endswith('absent-bus'), 'Run inside isolated runtime sandbox'
models={'parakeet':'parakeet-tdt-0.6b-v3-Q8_0.gguf','cohere':'cohere-transcribe-03-2026-Q4_K_M.gguf'}
legacy={'parakeet':'parakeet-tdt-0.6b-v2','cohere':'cohere-transcribe-int8'}
results=[]
with tempfile.TemporaryDirectory(dir=evidence) as temporary:
 root=pathlib.Path(temporary); config=root/'config.toml'; data=root/'data'; store=data/'voxtype/models'; store.mkdir(parents=True)
 for name in models.values():
  source=a.models/name; assert source.is_file(),str(source); (store/name).symlink_to(source)
 env={k:v for k,v in os.environ.items() if not k.startswith('VOXTYPE_')}
 env.update(HOME=str(root),XDG_CONFIG_HOME=str(root/'config'),XDG_DATA_HOME=str(data),XDG_CACHE_HOME=str(root/'cache'),TERM='xterm-256color')
 original='# preserve header\nengine = "whisper" # engine\n[parakeet]\nmodel = "parakeet-tdt-0.6b-v2" # pinned model\nmodel_type = "tdt"\nstreaming = false\non_demand_loading = false\n[cohere]\nmodel = "cohere-transcribe-int8" # sibling\nlanguage = "en"\ngguf_backend = "vulkan"\n[whisper]\nlanguage = "de"\n[profiles.keep]\nmodel = "custom-checkpoint"\n'
 def run(name,args,success=True,reset=True,extra=None):
  if reset: config.write_text(original)
  before=config.read_bytes(); current=dict(env); current.update(extra or {})
  x=subprocess.run([str(binary),'--config',str(config)]+args,env=current,capture_output=True,text=True,timeout=35)
  d=evidence/name; d.mkdir(exist_ok=True); (d/'stdout').write_text(x.stdout); (d/'stderr').write_text(x.stderr); (d/'exit').write_text(str(x.returncode)+'\n'); (d/'before.toml').write_bytes(before); (d/'after.toml').write_bytes(config.read_bytes())
  assert (x.returncode==0)==success,(name,x.returncode,x.stdout,x.stderr)
  if not success: assert config.read_bytes()==before,(name,'rejected selection mutated config')
  results.append({'case':name,'exit':x.returncode,'passed':True})
  return x
 try:
  for engine,model in models.items():
   run(engine+'-engine-switch',['config','set','engine',engine])
   saved=tomllib.loads(config.read_text()); assert saved['engine']==engine and saved[engine]['model']==model
   assert '# pinned model' in config.read_text() and '# sibling' in config.read_text()
   other='cohere' if engine=='parakeet' else 'parakeet'; assert saved[other]==tomllib.loads(original)[other]
   run(engine+'-validate',['config','validate-selection'],reset=False)
   config.write_text(original.replace('engine = "whisper"',f'engine = "{engine}"'))
   route=run(engine+'-invalid-saved-runtime-route',['transcribe',str(pathlib.Path(__file__).resolve().parents[1]/'tests/fixtures/vad/speech_hello.wav')],False,reset=False)
   assert 'ONNX' in route.stderr+route.stdout and 'GGUF' in route.stderr+route.stdout
   run(engine+'-reject-explicit-onnx',['config','set',engine+'.model',legacy[engine]],False)
   run(engine+'-reject-missing',['config','set',engine+'.model',str(root/'missing.gguf')],False)
   run(engine+'-reject-missing-store',['config','set','engine',engine],False,extra={'XDG_DATA_HOME':str(root/'empty-data')})
   run(engine+'-reject-worker',['config','set','engine',engine],False,extra={'VOXTYPE_GGUF_WORKER':str(root/'absent-worker')})
   custom=root/(engine+' custom.GGUF'); custom.symlink_to(a.models/model)
   run(engine+'-custom-path',['config','set',engine+'.model',str(custom)])
   run(engine+'-preserve-custom',['config','set','engine',engine],reset=False)
   assert tomllib.loads(config.read_text())[engine]['model']==str(custom)
   run(engine+'-setup-missing',['setup','--quiet','--no-post-install','--model',model,'--activate'],False,extra={'XDG_DATA_HOME':str(root/'empty-data')})
   run(engine+'-setup-unsupported',['setup','--quiet','--no-post-install','--download','--model',legacy[engine],'--activate'],False)
   run(engine+'-setup-activate',['setup','--quiet','--no-post-install','--model',model,'--activate'])
   assert tomllib.loads(config.read_text())[engine]['model']==model
   x=run(engine+'-picker',['info','models','--engine',engine,'--json'])
   entries=json.loads(x.stdout)['engines'][engine]['models']; assert entries and all(entry['name'].endswith('.gguf') for entry in entries)
  run('restart-compatible-selection',['config','set','engine','cohere'])
  runtime=pathlib.Path(env['XDG_RUNTIME_DIR'])/'voxtype';runtime.mkdir(parents=True,exist_ok=True)
  sentinel=subprocess.Popen(['/usr/bin/sleep','20'])
  try:
   (runtime/'voxtype.lock').write_text(str(sentinel.pid))
   for state in ['recording','transcribing','outputting']:
    (runtime/'state').write_text(state)
    run('restart-refuses-'+state,['config','validate-selection','--require-idle'],False,reset=False)
   (runtime/'state').write_text('idle')
   run('restart-idle',['config','validate-selection','--require-idle'],reset=False)
   (runtime/'meeting_state').write_text('recording\nfixture-meeting')
   run('restart-refuses-meeting',['config','validate-selection','--require-idle'],False,reset=False)
   (runtime/'meeting_state').unlink();(runtime/'state').unlink()
   run('restart-refuses-unknown-state',['config','validate-selection','--require-idle'],False,reset=False)
  finally:
   sentinel.terminate();sentinel.wait();(runtime/'voxtype.lock').unlink()
  if a.tui:
   for engine in models:
    config.write_text(original); before=config.read_bytes(); pid,fd=pty.fork()
    if pid==0: os.execve(str(binary),[str(binary),'--config',str(config),'configure'],env)
    fcntl.ioctl(fd,termios.TIOCSWINSZ,struct.pack('HHHH',42,180,0,0)); raw=bytearray()
    def drain(seconds=.5):
     until=time.monotonic()+seconds
     while time.monotonic()<until:
      if select.select([fd],[],[],.05)[0]:
       chunk=os.read(fd,65536); assert chunk,'TUI exited'; raw.extend(chunk)
    def send(keys): os.write(fd,keys.encode()); drain()
    try:
     drain(2); send('j\r'+('l' if engine=='parakeet' else 'll')+'s')
     assert config.read_bytes()==before,'TUI saved incompatible intermediate engine/model'
     assert b'ONNX' in raw and b'GGUF' in raw
     send('j'+('ll' if engine=='parakeet' else 'l')+'s')
     saved=tomllib.loads(config.read_text()); assert saved['engine']==engine and saved[engine]['model']==models[engine],saved
     (evidence/('tui-'+engine+'.toml')).write_bytes(config.read_bytes())
     results.append({'case':'tui-'+engine,'passed':True,'scope':'real PTY keyboard/save; child killed, no quit/restart'})
    finally:
     (evidence/('tui-'+engine+'.raw')).write_bytes(raw); os.kill(pid,signal.SIGKILL); os.waitpid(pid,0); os.close(fd)
 finally:
  (evidence/'results.json').write_text(json.dumps(results,indent=2)+'\n')
print(json.dumps({'passed':len(results),'failed':0,'cases':results},indent=2))
