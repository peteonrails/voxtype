#!/usr/bin/env python3
"""Archive source-matched desktop assets; never install or contact a shell."""
import argparse,gzip,hashlib,io,json,pathlib,re,tarfile

def main():
 parser=argparse.ArgumentParser(description=__doc__)
 parser.add_argument('--output',required=True,type=pathlib.Path)
 args=parser.parse_args()
 root=pathlib.Path(__file__).resolve().parents[1]
 match=re.search(r'^version = "([^"]+)"$',(root/'Cargo.toml').read_text(),re.M)
 if match is None:raise SystemExit('Missing package version in Cargo.toml')
 version=match.group(1)
 prefix=f'voxtype-{version}-desktop-assets/'
 paths=[]
 for name in ['omarchy-plugin','quickshell','examples/osd-packages','examples/osd-recipes','packaging/scripts']:
  paths.extend(p for p in (root/name).rglob('*') if p.is_file() and not p.is_symlink() and not any(part.startswith('.') or part=='__pycache__' for part in p.relative_to(root).parts))
 paths=sorted(set(paths))
 inventory={str(p.relative_to(root)):hashlib.sha256(p.read_bytes()).hexdigest() for p in paths}
 args.output.parent.mkdir(parents=True,exist_ok=True)
 with args.output.open('wb') as raw,gzip.GzipFile(fileobj=raw,filename='',mode='wb',mtime=0) as compressed,tarfile.open(fileobj=compressed,mode='w') as archive:
  def add(name,data,mode):
   info=tarfile.TarInfo(prefix+name);info.size=len(data);info.mode=mode
   archive.addfile(info,io.BytesIO(data))
  for path in paths:add(str(path.relative_to(root)),path.read_bytes(),0o755 if path.stat().st_mode&0o111 else 0o644)
  add('inventory.json',(json.dumps(inventory,indent=2,sort_keys=True)+'\n').encode(),0o644)
 print(json.dumps({'archive':str(args.output),'version':version,'files':len(inventory),'sha256':hashlib.sha256(args.output.read_bytes()).hexdigest()}))

if __name__=='__main__':main()
