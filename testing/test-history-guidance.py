"""NAS-only actual setup/help history guidance; no audio/output/helper endpoints."""
import hashlib
import json
import os
from pathlib import Path
import select
import shutil
import socket
import subprocess
import tempfile

assert os.getuid() == 65532 and Path('/work/out').is_dir()
assert not any(os.environ.get(k) for k in ['DISPLAY','WAYLAND_DISPLAY','SWAYSOCK','HYPRLAND_INSTANCE_SIGNATURE','DBUS_SESSION_BUS_ADDRESS'])
os.umask(0o077)
exe = Path(os.environ['VOXTYPE_HISTORY_TEST_BIN'])
sha = hashlib.sha256(exe.read_bytes()).hexdigest()
records = []
with tempfile.TemporaryDirectory(prefix='history-guidance-', dir='/work/scratch') as name:
    root = Path(name)
    config = root/'chosen.toml'
    default = root/'config/voxtype/config.toml'
    default.parent.mkdir(parents=True)
    default.write_text('# untouched default sentinel\n[history]\nenabled=true\n')
    before = default.read_bytes()
    tools = root/'tools'
    tools.mkdir()
    actual_which = shutil.which('which')
    assert actual_which
    (tools/'which').symlink_to(Path(actual_which).resolve())
    audio = socket.socket(socket.AF_UNIX)
    audio.bind(str(root/'audio.sock')); audio.listen(1); audio.setblocking(False)
    env = dict(HOME=str(root), XDG_CONFIG_HOME=str(root/'config'), XDG_DATA_HOME=str(root/'data'),
               XDG_RUNTIME_DIR=str(root/'runtime'), TMPDIR=str(root), PATH=str(tools), LC_ALL='C.UTF-8',
               DBUS_SESSION_BUS_ADDRESS='unix:path='+str(root/'absent-bus'), PULSE_SERVER='unix:'+str(root/'audio.sock'))
    def call(tail):
        command = [str(exe),'--config',str(config),*tail]
        p = subprocess.run(command, env=env, capture_output=True, text=True, timeout=15)
        record = dict(argv=command, rc=p.returncode, stdout=p.stdout, stderr=p.stderr)
        records.append(record)
        print('ACTUAL_HISTORY_GUIDANCE_COMMAND', json.dumps(record), flush=True)
        assert p.returncode == 0, record
        return p.stdout
    config.write_text('# Old explicit private config\n[hotkey]\nenabled=false\n')
    try:
        for action in [None,'list','search','show','copy','paste','correct','delete','clear','pause','resume','enable','disable','status','reindex','view']:
            text = call(['history',*([action] if action else []),'--help'])
            assert '--json' in text, ('Every actual history help must advertise JSON', action, text)
        for label, text, expected in [
            ('off', '# Old explicit private config\n[hotkey]\nenabled=false\n', ['Dictation History:', 'History is off', 'history enable', 'Enable History']),
            ('paused', '[history]\nenabled=true\npaused=true\n', ['Dictation History:', 'History is paused', 'history resume']),
            ('on', '[history]\nenabled=true\npaused=false\n', ['Dictation History:', 'History is on', 'history pause'])
        ]:
            config.write_text(text)
            output = call(['setup','check'])
            assert config.read_text() == text and default.read_bytes() == before, 'Diagnostic changed a private config'
            for marker in expected: assert marker in output, ('Missing actual setup history guidance',label,marker,output)
            assert '--config' in output and str(config) in output, 'Advice must preserve selected file; followed-command cases are tested separately'
            assert 'unknown app' in output and 'pause' in output.lower(), 'Do not overpromise app exclusions'
        skill = (Path(__file__).resolve().parents[1]/'skills/voxtype-setup/SKILL.md').read_text()
        assert 'Until then, the post-process command can keep a history.' not in skill, 'Stale stopgap advice contradicts builtin history'
        assert 'does not honor history pause or app exclusions' in ' '.join(skill.split()), 'Legacy logger needs an explicit separate privacy warning'
        assert not select.select([audio],[],[],0)[0], 'Guidance must not probe audio'
        assert hashlib.sha256(exe.read_bytes()).hexdigest() == sha
        print('ACTUAL_HISTORY_GUIDANCE_PASS', flush=True)
    finally:
        audio.close()
        Path('/work/out/history-guidance.json').write_text(json.dumps(dict(native_sha256=sha,records=records,unselected_sentinel_unchanged=default.read_bytes()==before,limits='Actual CLI help/diagnostics and source guidance only; missing models/output tools are expected diagnostic failures, not system readiness.'),indent=2)+'\n')
