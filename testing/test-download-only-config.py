#!/usr/bin/env python3
"""Real download-only with existing weights, absent and existing configs.
No network transfer: namespace is offline and supplied weights are already valid.
"""
import os,pathlib,subprocess,tempfile
assert os.environ.get('RC2_DESKTOP_ISOLATED')=='1'
assert pathlib.Path('/proc/1/comm').read_text().strip()=='bwrap'
with tempfile.TemporaryDirectory(dir='/evidence') as temp:
    r=pathlib.Path(temp);store=r/'data/voxtype/models';store.mkdir(parents=True)
    (store/'Qwen3-ASR-0.6B-Q8_0.gguf').symlink_to('/models/Qwen3-ASR-0.6B-Q8_0.gguf')
    env=dict(os.environ,HOME=str(r/'home'),XDG_CONFIG_HOME=str(r/'config'),XDG_DATA_HOME=str(r/'data'),XDG_CACHE_HOME=str(r/'cache'))
    config=r/'config/voxtype/config.toml'
    def download(model,success):
        p=subprocess.run(['/bundle/voxtype-vulkan','setup','--download','--no-post-install','--model',model],env=env,capture_output=True,text=True,timeout=30)
        print(p.returncode,p.stdout,p.stderr,flush=True)
        assert (p.returncode==0)==success
    download('Qwen3-ASR-0.6B-Q8_0.gguf',True)
    assert not config.exists(),'download-only created default config without Use'
    download('unknown-model-fixture',False)
    assert not config.exists(),'failed download created default config'
    config.parent.mkdir(parents=True,exist_ok=True)
    original='# preserve explicit selection\nengine = "parakeet"\n[parakeet]\nmodel = "/models/parakeet-ultra-Q4_K_M.gguf" # keep\n'
    config.write_text(original)
    download('Qwen3-ASR-0.6B-Q8_0.gguf',True)
    assert config.read_text()==original
    print('DOWNLOAD_ONLY_CONFIG_PASS')
