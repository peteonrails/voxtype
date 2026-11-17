#!/usr/bin/env python3
"""Repair must still target its pinned v3 model when Ultra is installed."""
import argparse,importlib.util,json,os,pathlib,shutil,tempfile
p=argparse.ArgumentParser();p.add_argument('--bundle',required=True,type=pathlib.Path);p.add_argument('--models',required=True,type=pathlib.Path);p.add_argument('--out',required=True,type=pathlib.Path);a=p.parse_args();a.out.mkdir(parents=True,exist_ok=True)
assert os.geteuid()!=0
s=importlib.util.spec_from_file_location('installer',a.bundle/'install-local.py');assert s and s.loader;m=importlib.util.module_from_spec(s);s.loader.exec_module(m)
with tempfile.TemporaryDirectory(dir=a.out) as d:
 home=pathlib.Path(d);os.environ['HOME']=str(home);store=home/'.local/share/voxtype/models';store.mkdir(parents=True)
 for name in ['parakeet-tdt-0.6b-v3-Q8_0.gguf','parakeet-ultra-Q8_0.gguf']:shutil.copyfile(a.models/name,store/name)
 original=b'# retain\nengine="parakeet"\n[parakeet]\nmodel="parakeet-tdt-0.6b-v2" # broken\nstreaming=false\n[tray]\nenabled=false\n'
 state=home/'state';state.mkdir();proposed=m.stage_selection(a.bundle.resolve(),state,original)
 assert b'parakeet-tdt-0.6b-v3-Q8_0.gguf' in proposed and b'# broken' in proposed
 (a.out/'proposed.toml').write_bytes(proposed)
 print(json.dumps({'passed':True,'repair_remains_v3_not_ultra':True,'production_mutation':False}))
