#!/usr/bin/env python3
"""Verify automatic packaged Vulkan-worker routing and real Cohere inference."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import time

p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--bundle', type=Path, required=True)
p.add_argument('--evidence', type=Path, required=True)
p.add_argument('--application-name', default='voxtype-vulkan')
p.add_argument('--worker-name', default='voxtype-gguf-worker-vulkan')
p.add_argument('--baseline-only', action='store_true')
a = p.parse_args()
bundle = a.bundle.resolve(); out = a.evidence.resolve(); out.mkdir(parents=True, exist_ok=True)
repo = Path(__file__).resolve().parents[1]
model = Path('/home/pete/.local/share/voxtype/models/cohere-transcribe-03-2026-Q4_K_M.gguf')
assert model.is_file()
assert (bundle / 'voxtype').is_symlink() and (bundle / 'voxtype').resolve() == bundle / a.application_name
worker = bundle / a.worker_name
env = {k:v for k,v in os.environ.items() if not k.startswith('VOXTYPE_')}
results = []
for name, enabled, fixture, expected in [
    ('baseline',False,'speech_hello.wav','hello world'),
    ('missing-lard',True,'speech_hello.wav','hello world'),
    ('corrupt-lard',True,'speech_hello.wav','hello world'),
    ('longer',False,'speech_long.wav','this is a longer test of voice activity detection with multiple words and phrases'),
]:
    if a.baseline_only and name != 'baseline': continue
    lard = out / ('corrupt.vxdf' if name == 'corrupt-lard' else 'missing.vxdf')
    if name == 'corrupt-lard': lard.write_bytes(b'invalid isolated test model, not production weights')
    config = out / (name+'.toml')
    config.write_text('engine = "cohere"\n[cohere]\nmodel = '+json.dumps(str(model))+'\nlanguage = "en"\ngguf_backend = "vulkan"\nthreads = 2\n[text]\nfilter_filler_words = false\nlard_disfluency = '+str(enabled).lower()+'\nlard_model_path = '+json.dumps(str(lard))+'\n')
    cmd = [str(bundle/'voxtype'),'--config',str(config),'transcribe',str(repo/'tests/fixtures/vad'/fixture)]
    stdout = (out/(name+'.stdout')).open('w'); stderr = (out/(name+'.stderr')).open('w')
    proc = subprocess.Popen(cmd,env=env,stdout=stdout,stderr=stderr)
    identities = []
    try:
        end = time.monotonic()+120
        while proc.poll() is None and time.monotonic()<end:
            children = Path('/proc')/str(proc.pid)/'task'/str(proc.pid)/'children'
            try:
                for pid in children.read_text().split():
                    try:
                        exe = str((Path('/proc')/pid/'exe').resolve(strict=True))
                        if exe not in [x['exe'] for x in identities]: identities.append({'pid':int(pid),'exe':exe})
                    except (FileNotFoundError,ProcessLookupError): pass
            except (FileNotFoundError,ProcessLookupError): pass
            time.sleep(.005)
        if proc.poll() is None: proc.kill(); raise AssertionError('inference timeout')
    finally:
        proc.wait(timeout=5); stdout.close(); stderr.close()
    sout=(out/(name+'.stdout')).read_text(); serr=(out/(name+'.stderr')).read_text()
    transcript=sout.strip().splitlines()[-1] if sout.strip() else ''
    record={'name':name,'command':cmd,'exit':proc.returncode,'transcript':transcript,'children':identities,'worker_override':None}
    results.append(record)
    (out/'partial-result.json').write_text(json.dumps(results,indent=2)+'\n')
    assert proc.returncode==0, record
    assert re.findall(r'\w+',transcript.casefold())==expected.split(), transcript
    assert re.search(r'\[Vulkan\d+\].*AMD Radeon RX 7800 XT',sout+serr), serr
    assert str(worker) in [x['exe'] for x in identities], identities
    assert ('LARD cleanup unavailable' in sout+serr) is enabled
manifest={'binary':str((bundle/'voxtype').resolve()),'binary_sha256':hashlib.sha256((bundle/'voxtype').read_bytes()).hexdigest(),'worker_sha256':hashlib.sha256(worker.read_bytes()).hexdigest(),'cases':results}
(out/'result.json').write_text(json.dumps(manifest,indent=2)+'\n')
print('COMBINED_VULKAN_PASS:', len(results), 'automatic packaged-worker Cohere cases, actual /proc child identities, RX 7800 XT Vulkan0, expected words; fail-open checked when applicable')
