#!/usr/bin/env python3
"""Real CLI atomic selection. Run only through runtime-isolate.py.
The additive command is intentionally generic GGUF only, not a universal picker.
All GGUF choices use preexisting real model files, with no model transfer.
"""
import json, os, pathlib, subprocess, tempfile, tomllib
assert os.environ.get('RC2_DESKTOP_ISOLATED') == '1'
assert pathlib.Path('/proc/1/comm').read_text().strip() == 'bwrap'
binary = '/bundle/voxtype-vulkan'
results=[]
with tempfile.TemporaryDirectory(dir='/evidence') as temp:
    root = pathlib.Path(temp)
    config = root / 'config.toml'
    original = '# selection transaction\nengine = "parakeet"\n[parakeet]\nmodel = "/models/parakeet-ultra-F16.gguf" # preserve\n[gguf]\nlanguage = "de" # hint\ngguf_backend = "vulkan"\n'
    config.write_text(original)
    env = dict(os.environ, XDG_DATA_HOME=str(root/'data'), VOXTYPE_GGUF_WORKER='/bundle/voxtype-gguf-worker-vulkan')
    def select(engine, model, *extra, success=True):
        before=config.read_bytes()
        p = subprocess.run([binary, '--config', str(config), 'config', 'select-model', engine, str(model), *extra], env=env, cwd=root, capture_output=True, text=True)
        print(engine, str(model), p.returncode, p.stdout, p.stderr, flush=True)
        assert (p.returncode == 0) == success, p.stderr
        if not success:assert config.read_bytes()==before,'failed transaction changed config'
        results.append({'engine':engine,'model':str(model),'expected_success':success,'exit':p.returncode,'passed':True})
        return p
    select('gguf', '/models/Qwen3-ASR-0.6B-Q8_0.gguf')
    saved = tomllib.loads(config.read_text())
    assert saved['engine'] == 'gguf'
    assert saved['gguf']['architecture'] == 'qwen3_asr'
    assert saved['gguf']['language'] == 'de'
    assert saved['gguf']['gguf_backend'] == 'vulkan'
    assert '# preserve' in config.read_text() and '# hint' in config.read_text()
    for engine in ['whisper','parakeet','moonshine','sensevoice','paraformer','dolphin','omnilingual','cohere','gguf','openvino']:
        select(engine, '/missing.gguf', success=False)
    select('cohere', '/models/parakeet-ultra-Q4_K_M.gguf', success=False)
    select('parakeet', 'parakeet-tdt-0.6b-v2', success=False)
    select('gguf','/models/Qwen3-ASR-0.6B-Q8_0.gguf','--language','invalid-language',success=False)
    env['VOXTYPE_GGUF_WORKER']='/missing-worker'
    select('gguf','/models/Qwen3-ASR-0.6B-Q8_0.gguf',success=False)
    env['VOXTYPE_GGUF_WORKER']='/bundle/voxtype-gguf-worker-vulkan'
    custom=root/'custom.gguf';custom.symlink_to('/models/Qwen3-ASR-0.6B-Q8_0.gguf')
    select('gguf',custom)
    select('gguf','./'+custom.name)
    # The worker prefers cwd files, then the model store, even for relative paths.
    select('gguf',custom.name)
    store=root/'data/voxtype/models';store.mkdir(parents=True,exist_ok=True)
    (store/custom.name).symlink_to('/models/Qwen3-ASR-0.6B-Q8_0.gguf')
    custom.unlink();custom.write_bytes(b'not a GGUF checkpoint')
    select('gguf',custom.name,success=False)
    custom.unlink();custom.symlink_to('/models/Qwen3-ASR-0.6B-Q8_0.gguf')
    (store/'nested').mkdir()
    (store/'nested/custom.gguf').symlink_to('/models/Qwen3-ASR-0.6B-Q8_0.gguf')
    select('gguf','nested/custom.gguf')
    assert tomllib.loads(config.read_text())['gguf']['language']=='de'
    # A language change is explicit and atomic, never made while just browsing.
    select('gguf','/models/gigaam-v3-e2e-rnnt-Q8_0.gguf',success=False)
    select('gguf','/models/gigaam-v3-e2e-rnnt-Q8_0.gguf','--language','ru')
    assert tomllib.loads(config.read_text())['gguf']['language']=='ru'
    for model, language, architecture in [
        ('Qwen3-ASR-1.7B-Q8_0.gguf','de','qwen3_asr'),
        ('moonshine-streaming-small-Q8_0.gguf','en','moonshine_streaming'),
        ('nemotron-3.5-asr-streaming-0.6b-Q8_0.gguf','en-US','parakeet'),
    ]:
        select('gguf','/models/'+model,'--language',language)
        saved=tomllib.loads(config.read_text())
        assert saved['gguf']['architecture']==architecture
        assert saved['gguf']['language']==language
        assert saved['gguf']['model']=='/models/'+model
    select('gguf','/models/nemotron-3.5-asr-streaming-0.6b-Q8_0.gguf','--language','en',success=False)
    assert saved['gguf']['gguf_backend']=='vulkan'
    assert saved['parakeet']==tomllib.loads(original)['parakeet']
    pathlib.Path('/evidence/results.json').write_text(json.dumps(results,indent=2))
    pathlib.Path('/evidence/after.toml').write_text(config.read_text())
    print('TRANSACTION_PASS')
