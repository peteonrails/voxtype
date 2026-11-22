"""Measure the final readonly CLI/GTK pair around an actual NAS fixture.
Does not replace commands, responses, outcomes or assertions in the fixture.
"""
import hashlib,json,os,runpy,sys
from pathlib import Path
assert os.getuid()==65532
assert len(sys.argv)==2
fixture=Path(sys.argv[1]);assert fixture.is_file() and fixture.parent==Path('testing')
def identity():
 result={}
 for key in ['VOXTYPE_HISTORY_TEST_BIN','VOXTYPE_HISTORY_TEST_GTK_BIN']:
  p=Path(os.environ[key]);st=p.stat()
  assert st.st_uid==0 and not st.st_mode&0o222 and not os.access(p,os.W_OK)
  result[key]=dict(path=str(p),sha256=hashlib.sha256(p.read_bytes()).hexdigest(),uid=st.st_uid,mode=oct(st.st_mode&0o777))
 return result
before=identity();print('FINAL_FROZEN_NATIVE_BEFORE',json.dumps(before,sort_keys=True),flush=True)
sys.argv=[str(fixture)]
try:runpy.run_path(str(fixture),run_name='__main__')
finally:
 after=identity();print('FINAL_FROZEN_NATIVE_AFTER',json.dumps(after,sort_keys=True),flush=True)
 assert before==after,'Native bytes/authority changed during actual fixture'
