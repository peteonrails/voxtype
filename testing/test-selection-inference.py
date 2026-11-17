#!/usr/bin/env python3
"""Recording-free packaged automatic-worker Vulkan inference on known WAVs."""
import argparse, hashlib, json, os, pathlib, re, subprocess, time
p=argparse.ArgumentParser(description=__doc__);p.add_argument('--bin',required=True,type=pathlib.Path);p.add_argument('--models',required=True,type=pathlib.Path);p.add_argument('--evidence',required=True,type=pathlib.Path);p.add_argument('--fixtures',type=pathlib.Path);a=p.parse_args()
r=pathlib.Path(__file__).resolve().parents[1];e=a.evidence.resolve();e.mkdir(parents=True,exist_ok=True);binary=a.bin.resolve();fixtures=(a.fixtures or r/'tests/fixtures/vad').resolve();results=[]
models={'parakeet':'parakeet-tdt-0.6b-v3-Q8_0.gguf','cohere':'cohere-transcribe-03-2026-Q4_K_M.gguf'}
expected={'speech_hello.wav':'hello world','speech_long.wav':'this is a longer test of voice activity detection with multiple words and phrases'}
for engine,model in models.items():
 store=e/'model-data/voxtype/models';store.mkdir(parents=True,exist_ok=True)
 if not (store/model).exists():(store/model).symlink_to(a.models/model)
 env={k:v for k,v in os.environ.items() if not k.startswith('VOXTYPE_')};env['XDG_DATA_HOME']=str(e/'model-data')
 legacy='parakeet-tdt-0.6b-v2' if engine=='parakeet' else 'cohere-transcribe-int8'
 config=e/(engine+'.toml');config.write_text(f'# installed-like retained legacy table\nengine="whisper"\n[{engine}]\nmodel={json.dumps(legacy)} # preserve\ngguf_backend="vulkan"\nthreads=2\n'+('language="en"\n' if engine=='cohere' else 'streaming=false\nmodel_type="tdt"\n'))
 selection=subprocess.run([str(binary),'--config',str(config),'config','set','engine',engine],env=env,capture_output=True,text=True,timeout=20)
 (e/(engine+'-selection.stdout')).write_text(selection.stdout);(e/(engine+'-selection.stderr')).write_text(selection.stderr);(e/(engine+'-selection.exit')).write_text(str(selection.returncode)+'\n');assert selection.returncode==0,(selection.stdout,selection.stderr)
 for fixture,words in expected.items():
  name=engine+'-'+fixture;out=e/(name+'.stdout');err=e/(name+'.stderr');children={};start=time.monotonic()
  with out.open('w') as stdout,err.open('w') as stderr:
   child=subprocess.Popen([str(binary),'-vv','--config',str(config),'transcribe',str(fixtures/fixture)],env=env,stdout=stdout,stderr=stderr)
   try:
    while child.poll() is None:
     if time.monotonic()-start>120:raise TimeoutError(name)
     path=pathlib.Path(f'/proc/{child.pid}/task/{child.pid}/children')
     try:pids=path.read_text().split()
     except FileNotFoundError:pids=[]
     for pid in pids:
      if pid in children:continue
      try:
       exe=pathlib.Path(f'/proc/{pid}/exe');real=str(exe.resolve());digest=hashlib.sha256(exe.read_bytes()).hexdigest();children[pid]={'path':real,'sha256':digest}
      except (FileNotFoundError,PermissionError):pass
     time.sleep(.05)
   finally:
    if child.poll() is None:child.kill()
    child.wait()
  (e/(name+'.exit')).write_text(str(child.returncode)+'\n');text=out.read_text();logs=err.read_text();lines=[line.strip() for line in text.splitlines() if line.strip()];transcript=lines[-1] if lines else '';normalized=' '.join(re.findall(r"[a-z]+",transcript.lower()))
  result={'case':name,'exit':child.returncode,'transcript':transcript,'normalized':normalized,'expected':words,'worker_processes':children,'seconds':time.monotonic()-start}
  (e/(name+'.json')).write_text(json.dumps(result,indent=2)+'\n');results.append(result);(e/'results.json').write_text(json.dumps(results,indent=2)+'\n')
  assert child.returncode==0,(name,text,logs)
  assert normalized==words,(name,transcript,words)
  assert any('gguf-worker' in item['path'] for item in children.values()),children
  assert re.search(r'Vulkan\d|\[vulkan\]',text+logs,re.I) and re.search(r'RX\s*7800|RADV\s*NAVI\s*32',text+logs,re.I),(name,'no actual RX7800XT Vulkan load evidence',text,logs)
print(json.dumps({'passed':len(results),'failed':0,'results':results},indent=2))
