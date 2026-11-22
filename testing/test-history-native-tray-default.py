"""Frozen222 real tray fixture, additionally exercising implicit private default.
Only fixture config placement/daemon argv and authority labels are adapted.
Native tray, installed private QML and canonical actions remain unchanged.
"""
import hashlib,os
from pathlib import Path
base=Path(__file__).with_name('test-history-native-tray.py');data=base.read_bytes()
assert hashlib.sha256(data).hexdigest()=='faf0b3a1be14b98da9ae55c9bb5609ac30f2664fb5fa16c801a32521fe40dfa8'
assert os.environ['VOX_HISTORY_TRAY_CASE']=='default'
code=data.decode()
changes=[("case in ['explicit','legacy']","case in ['explicit','legacy','default']"),("config=root/'chosen.toml'","config=default\n default=root/'unchosen.toml';default.write_text('# Untouched unchosen sentinel\\n[history]\\nenabled=false\\n')"),("daemon=start([str(exe),'--config',str(config)],'daemon')","daemon=start([str(exe)],'daemon')"),("digest=hashlib.sha256(Path('/proc',str(daemon.pid),'exe').read_bytes()).hexdigest();assert digest==hashlib.sha256(exe.read_bytes()).hexdigest()","digest=hashlib.sha256(Path('/proc',str(daemon.pid),'exe').read_bytes()).hexdigest();assert digest==hashlib.sha256(exe.read_bytes()).hexdigest()\n  native_argv=Path('/proc',str(daemon.pid),'cmdline').read_bytes().decode().strip('\\0').split('\\0');assert native_argv==[str(exe)],native_argv"),("chosen_config=str(config),default_unchanged=True","intended_config=str(config),daemon_argv=native_argv,unselected_sentinel_unchanged=True")]
for old,new in changes:assert code.count(old)==1,old;code=code.replace(old,new)
print('PRIVATE_DEFAULT_TRAY_FIXTURE_ADAPTER',hashlib.sha256(data).hexdigest(),hashlib.sha256(code.encode()).hexdigest(),'native daemon without --config, private XDG default, unchosen sentinel',flush=True)
exec(compile(code,str(base),'exec'),{'__name__':'__main__','__file__':str(base)})
