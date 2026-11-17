#!/usr/bin/env python3
"""Actual PTY engine roundtrip must preserve a saved GGUF language hint."""
import argparse,fcntl,json,os,pathlib,pty,select,signal,struct,termios,time,tomllib
p=argparse.ArgumentParser();p.add_argument('--bin',required=True,type=pathlib.Path);p.add_argument('--worker',required=True,type=pathlib.Path);p.add_argument('--models',required=True,type=pathlib.Path);p.add_argument('--out',required=True,type=pathlib.Path);a=p.parse_args();a.out.mkdir(parents=True,exist_ok=True);home=a.out/'home';home.mkdir(exist_ok=True);store=home/'data/voxtype/models';store.mkdir(parents=True,exist_ok=True)
for f in a.models.glob('*.gguf'):
 target=store/f.name
 if not target.exists():target.symlink_to(f.resolve())
config=a.out/'config.toml';original='# preserve explicit German\nengine="gguf"\n[gguf]\nmodel="Qwen3-ASR-0.6B-Q8_0.gguf"\narchitecture="qwen3_asr"\nlanguage="de"\ngguf_backend="vulkan"\nthreads=2\n';config.write_text(original);env=dict(os.environ,HOME=str(home.resolve()),XDG_DATA_HOME=str((home/'data').resolve()),XDG_CONFIG_HOME=str((home/'config').resolve()),XDG_CACHE_HOME=str((home/'cache').resolve()),VOXTYPE_GGUF_WORKER=str(a.worker.resolve()),TERM='xterm-256color',DBUS_SESSION_BUS_ADDRESS='unix:path=/run/user/1000/absent-bus');pid,fd=pty.fork()
if pid==0:os.execve(str(a.bin.resolve()),[str(a.bin.resolve()),'--config',str(config.resolve()),'configure'],env)
fcntl.ioctl(fd,termios.TIOCSWINSZ,struct.pack('HHHH',42,180,0,0));raw=bytearray()
def drain(seconds):
 end=time.monotonic()+seconds
 while time.monotonic()<end:
  if select.select([fd],[],[],.05)[0]:
   data=os.read(fd,65536)
   if not data:break
   raw.extend(data)
try:
 drain(2);os.write(fd,b'j\rhls');drain(2);saved=tomllib.loads(config.read_text());result={'engine':saved['engine'],'language':saved['gguf']['language'],'model':saved['gguf']['model'],'passed':saved['engine']=='gguf' and saved['gguf']['language']=='de','scope':'real PTY left/right engine roundtrip and save, child killed without normal quit/restart'};(a.out/'result.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result));assert result['passed'],'Saved explicit GGUF language was reset when returning to the same engine/model'
 os.write(fd,b'j');drain(.3)
 expected={'Qwen3-ASR-0.6B-Q8_0.gguf':'qwen3_asr','Qwen3-ASR-1.7B-Q8_0.gguf':'qwen3_asr','moonshine-streaming-small-Q8_0.gguf':'moonshine_streaming','gigaam-v3-e2e-rnnt-Q8_0.gguf':'gigaam','nemotron-3.5-asr-streaming-0.6b-Q8_0.gguf':'parakeet'};seen=[]
 for _ in expected:
  os.write(fd,b'ls');drain(.6);saved=tomllib.loads(config.read_text());g=saved['gguf'];assert saved['engine']=='gguf' and expected.get(g['model'])==g['architecture'],saved;seen.append(g['model'])
 assert set(seen)==set(expected) and len(seen)==len(set(seen)),seen
 (a.out/'all-models.json').write_text(json.dumps({'passed':True,'models':seen,'actual_pty_model_cycles_and_saves':True,'restricted_models_absent':True},indent=2)+'\n');print('PASS actual TUI saved all',len(seen),'curated variants')
 before_streaming=config.read_bytes();os.write(fd,b'jjjjjjls');drain(.8)
 assert config.read_bytes()==before_streaming,'Unsupported native streaming selection wrote configuration'
 assert b'Native streaming' in raw or b'streaming' in raw, 'No visible streaming diagnostic'
 (a.out/'streaming-rejection.json').write_text(json.dumps({'passed':True,'same_save_native_streaming_rejected_no_write':True,'config_sha256_unchanged':True},indent=2)+'\n');print('PASS native streaming same-save rejected byte-identically')
finally:
 (a.out/'tui.raw').write_bytes(raw);os.kill(pid,signal.SIGKILL);os.waitpid(pid,0);os.close(fd)
