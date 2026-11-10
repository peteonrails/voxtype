#!/usr/bin/env python3
"""Exercise real gio desktop launch, failed frontend fallback and bundled TUI.

The desktop frontend and terminal emulator are disposable fixtures, not a GTK app.
The terminal fixture actually starts the bundled configure process through a PTY.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import time

p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--bundle', type=Path, required=True)
p.add_argument('--evidence', type=Path, required=True)
a = p.parse_args()
bundle = a.bundle.resolve(); out = a.evidence.resolve(); out.mkdir(parents=True, exist_ok=True)
bin_dir = out / 'bin'; bin_dir.mkdir(exist_ok=True)
data = out / 'data/applications'; data.mkdir(parents=True, exist_ok=True)
run = out / 'runtime'; run.mkdir(mode=0o700, exist_ok=True)
for name in ('gio', 'grep', 'flock', 'timeout'):
    dest = bin_dir / name
    if not dest.exists(): dest.symlink_to('/usr/bin/' + name)
config = out / 'config.toml'; config.write_text('# launcher TUI fixture\n[hotkey]\nkey = "F13"\n')
terminal = bin_dir / 'terminal-fixture'
terminal.write_text('''#!/usr/bin/python3
import os,pty,subprocess,time,select,json,fcntl,termios,struct
from pathlib import Path
root=Path(os.environ['EVIDENCE'])
(root/'terminal-argv.json').write_text(json.dumps(__import__('sys').argv))
m,s=pty.openpty()
fcntl.ioctl(s,termios.TIOCSWINSZ,struct.pack('HHHH',40,140,0,0))
p=subprocess.Popen([os.environ['BUNDLE']+'/voxtype','--config',str(root/'config.toml'),'configure'],stdin=s,stdout=s,stderr=s,env=dict(os.environ,TERM='xterm-256color'),start_new_session=True)
os.close(s); raw=b''
try:
 end=time.monotonic()+2
 while time.monotonic()<end:
  if select.select([m],[],[],.1)[0]: raw+=os.read(m,65536)
 assert p.poll() is None,'TUI terminated unexpectedly'
 assert b'General' in raw and b'Cleanup' in raw,repr(raw[-400:])
 (root/'tui-started').write_text(str(p.pid))
finally:
 p.kill();p.wait(timeout=3);os.close(m)
 (root/'terminal.raw').write_bytes(raw)
''')
terminal.chmod(0o755)
app = bin_dir / 'graphical-settings-fixture'
app.write_text('#!/usr/bin/python3\nimport os\nfrom pathlib import Path\nPath(os.environ["EVIDENCE"],"graphical-started").write_text("real gio launched disposable graphical frontend fixture")\n')
app.chmod(0o755)
desktop = data / 'voxtype-gtk-fixture.desktop'
desktop.write_text('[Desktop Entry]\nType=Application\nName=Disposable Settings frontend\nCategories=Settings;\nTerminal=false\nExec=' + str(app) + '\n')
env = dict(os.environ, PATH=str(bin_dir), HOME=str(out), XDG_RUNTIME_DIR=str(run), XDG_DATA_HOME=str(out / 'data'), XDG_DATA_DIRS=str(out / 'none'), TERMINAL='terminal-fixture', BUNDLE=str(bundle), EVIDENCE=str(out))
# No omarchy-shell exists on this PATH: the real QS test is separate.
results = []
def launch(case):
    r = subprocess.run([str(bundle / 'voxtype-configure-launcher')], env=env, text=True, capture_output=True, timeout=8)
    results.append({'case':case,'exit':r.returncode,'stdout':r.stdout,'stderr':r.stderr})
    assert r.returncode == 0, results[-1]
launch('real-gio-desktop-fixture')
end = time.monotonic()+4
while not (out/'graphical-started').exists() and time.monotonic()<end: time.sleep(.05)
assert (out/'graphical-started').exists(), results
assert not (out/'tui-started').exists(), 'gio branch launched terminal too'
desktop.write_text(desktop.read_text().replace(str(app),str(out/'missing-graphical-app')))
launch('failed-real-gio-to-real-bundled-tui')
assert (out/'tui-started').exists(), results
args = json.loads((out/'terminal-argv.json').read_text())
assert args[1:] == ['-e','voxtype','configure'], args
assert config.read_text() == '# launcher TUI fixture\n[hotkey]\nkey = "F13"\n'
(out/'result.json').write_text(json.dumps(results,indent=2)+'\n')
print('COMBINED_FRONTENDS_PASS: real gio executes desktop fixture; broken executable falls back to terminal fixture running exact bundled TUI through PTY; no production service access')
