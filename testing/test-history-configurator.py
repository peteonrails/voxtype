"""Actual configure PTY privacy persistence in the private NAS sandbox only.

No recording, model inference, real desktop, normal-quit service restart, or
fabricated successful command. Native code stays unchanged during this fixture.
"""
import codecs
import fcntl
import hashlib
import json
import os
from pathlib import Path
import pty
import select
import socket
import struct
import subprocess
import tempfile
import termios
import time
import tomllib

import pyte
from PIL import Image, ImageDraw, ImageFont

assert os.getuid() == 65532 and Path('/work/out').is_dir()
assert not any(os.environ.get(k) for k in ['DISPLAY', 'WAYLAND_DISPLAY', 'SWAYSOCK', 'HYPRLAND_INSTANCE_SIGNATURE'])
os.umask(0o077)
case = os.environ.get('VOX_HISTORY_CONFIG_CASE', 'explicit')
assert case in ['explicit', 'default']
exe = Path(os.environ['VOXTYPE_HISTORY_TEST_BIN'])
native_sha = hashlib.sha256(exe.read_bytes()).hexdigest()
events = []
with tempfile.TemporaryDirectory(prefix='history-config-', dir='/work/scratch') as name:
    root = Path(name)
    default = root/'config/voxtype/config.toml'
    explicit = root/'chosen.toml'
    default.parent.mkdir(parents=True, mode=0o700)
    chosen, sentinel = (explicit, default) if case == 'explicit' else (default, explicit)
    chosen.write_text('# Private old config, do not opt in on load\n[whisper]\nmodel = "private-missing-model" # preserve model\n[text]\nfilter_filler_words = false # preserve unrelated\n')
    sentinel.write_text('# Unselected sentinel\n[history]\nenabled = true\npaused = true\nexclude_apps = ["unchosen"]\n')
    sentinel_before = sentinel.read_bytes()
    assert chosen.stat().st_mode & 0o777 == 0o600
    assert not Path(str(chosen)+'.tmp').exists()
    # Create the originals privately, then exercise the real desktop umask.
    os.umask(0o022)
    tools = root/'tools'
    tools.mkdir(mode=0o700)
    (tools/'voxtype').symlink_to(exe)
    audio = socket.socket(socket.AF_UNIX)
    audio.bind(str(root/'audio.sock'))
    audio.listen(4)
    audio.setblocking(False)
    env = dict(HOME=str(root), XDG_CONFIG_HOME=str(root/'config'), XDG_DATA_HOME=str(root/'data'),
               XDG_RUNTIME_DIR=str(root/'runtime'), TMPDIR=str(root), PATH=str(tools),
               TERM='xterm-256color', LC_ALL='C.UTF-8', DBUS_SESSION_BUS_ADDRESS='unix:path='+str(root/'absent-bus'),
               PULSE_SERVER='unix:'+str(root/'audio.sock'))
    args = [str(exe)] + (['--config', str(chosen)] if case == 'explicit' else [])
    generation_file = root/'data/voxtype/history-control'/hashlib.sha256(os.fsencode(chosen.resolve())).hexdigest()/'generation'

    def generation():
        if not generation_file.exists(): return 0
        assert generation_file.stat().st_mode & 0o777 == 0o600
        return int.from_bytes(generation_file.read_bytes(), 'little')

    def saved():
        assert sentinel.read_bytes() == sentinel_before, 'Wrong config target was mutated'
        assert '# preserve model' in chosen.read_text() and '# preserve unrelated' in chosen.read_text()
        assert chosen.stat().st_mode & 0o777 == 0o600, ('Config replacement widened permissions under022', oct(chosen.stat().st_mode & 0o777))
        return tomllib.loads(chosen.read_text())

    def command(*tail):
        result = subprocess.run(args+list(tail), env=env, capture_output=True, text=True, timeout=12)
        events.append(dict(argv=args+list(tail), rc=result.returncode, stdout=result.stdout, stderr=result.stderr))
        assert result.returncode == 0, events[-1]
        return json.loads(result.stdout)

    def open_form(label):
        master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 42, 160, 0, 0))
        child = subprocess.Popen(args+['configure'], env=env, stdin=slave, stdout=slave, stderr=slave, start_new_session=True)
        os.close(slave)
        screen = pyte.Screen(160, 42)
        stream = pyte.Stream(screen)
        decoder = codecs.getincrementaldecoder('utf-8')()
        raw = bytearray()

        def drain(seconds=.3):
            deadline = time.monotonic()+seconds
            while time.monotonic() < deadline:
                if select.select([master], [], [], min(.05, max(0, deadline-time.monotonic())))[0]:
                    chunk = os.read(master, 65536)
                    assert chunk and child.poll() is None, ('Early configure exit', child.returncode, bytes(raw))
                    raw.extend(chunk)
                    stream.feed(decoder.decode(chunk))
                    if b'\x1b[6n' in chunk: os.write(master, b'\x1b[1;1R')

        def wait(text):
            deadline = time.monotonic()+12
            while text not in '\n'.join(screen.display):
                assert time.monotonic() < deadline, (text, '\n'.join(screen.display), bytes(raw))
                drain(.1)

        def send(keys, reason):
            events.append(dict(input=reason))
            os.write(master, keys)
            drain()

        def snapshot(stage):
            drain()
            stem = Path('/work/out')/('history-config-'+case+'-'+stage)
            stem.with_suffix('.txt').write_text('\n'.join(screen.display)+'\n')
            image = Image.new('RGB', (160*9, 42*18), '#101218')
            draw = ImageDraw.Draw(image)
            font = ImageFont.truetype('/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf', 14)
            for y, line in enumerate(screen.display): draw.text((0, y*18), line, font=font, fill='#e4e4e7')
            image.save(stem.with_suffix('.png'))
            return '\n'.join(screen.display)

        try:
            wait('General')
            # Direct SGR mouse selection of Section::ALL[8], without visiting
            # Audio (sidebar arrow navigation opens every traversed section).
            send(b'\x1b[<0;3;10M\x1b[<0;3;10m', 'Direct private PTY mouse History sidebar row')
            wait('Collection, retention and privacy')
            send(b'\r', 'Focus History form, not a sibling section')
            wait('Dictation history')
            return master, child, screen, raw, drain, wait, send, snapshot
        except BaseException:
            child.kill(); child.wait(); os.close(master)
            Path('/work/out/history-config-'+case+'-'+label+'.ansi').write_bytes(raw)
            raise

    def close_form(handle, label):
        master, child, screen, raw, drain, wait, send, snapshot = handle
        if child.poll() is None: child.terminate()
        rc = child.wait(timeout=8)
        os.close(master)
        Path('/work/out/history-config-'+case+'-'+label+'.ansi').write_bytes(raw)
        events.append(dict(fixture_stopped_configure=True, actual_child_rc=rc, normal_quit_not_used=True))

    initial = command('history', 'status', '--json')
    assert not initial['enabled'] and not initial['paused']
    assert 'history' not in tomllib.loads(chosen.read_text())
    handle = open_form('first')
    master, child, screen, raw, drain, wait, send, snapshot = handle
    try:
        initial_frame = snapshot('initial')
        assert 'Pause history' in initial_frame and 'Record window titles' in initial_frame
        assert chosen.read_text().startswith('# Private old config') and 'history' not in tomllib.loads(chosen.read_text())
        send(b's', 'Unrelated no-op save must not enable old config')
        wait('Nothing to save')
        assert 'history' not in tomllib.loads(chosen.read_text())
        before_cancel = chosen.read_bytes()
        send(b'jj\r\x159\x1b', 'Cancel retention before any real save, ordinary022')
        send(b's', 'Cancelled first edit must leave no pending save')
        wait('Nothing to save')
        assert chosen.read_bytes() == before_cancel and chosen.stat().st_mode & 0o777 == 0o600
        rejected = subprocess.run(args+['config','set','history.retention_days','--','-1'], env=env, capture_output=True, text=True, timeout=10)
        assert rejected.returncode != 0 and chosen.read_bytes() == before_cancel and chosen.stat().st_mode & 0o777 == 0o600
        events.append(dict(rejected_edit_rc=rejected.returncode, bytes_and_private_mode_unchanged=True, umask='022'))
        send(b'kk', 'Return to enable after cancellation control')
        send(b' s', 'Enable only history and save')
        wait('Saved; takes effect now')
        history = saved()['history']
        assert history == {'enabled': True}, history
        assert generation() == 0
        send(b'j s', 'Pause and save through actual ConfigEditor')
        wait('Saved; takes effect now')
        assert saved()['history']['paused'] and generation() == 1
        assert command('history', 'status', '--json')['paused']
        send(b'j\r\x150\rs', 'Retention 0 forever; preserve pause')
        wait('Saved; takes effect now')
        assert saved()['history']['retention_days'] == 0 and saved()['history']['paused']
        send(b'j', 'Show focused-app exclusion privacy limitations')
        wait('Unsupported desktops')
        warning_frame = snapshot('warning')
        assert 'need an explicit' in warning_frame and 'pause before sensitive dictation.' in warning_frame and 'browser-hosted' in warning_frame
        send(b'\r\x15private-editor-q, Bitwarden\rs', 'Literal exclusion value with q must stay inside inline editor')
        wait('Saved; takes effect now')
        assert saved()['history']['exclude_apps'] == 'private-editor-q, Bitwarden'
        assert command('config', 'get', '--json')['history.exclude_apps'] == 'private-editor-q,Bitwarden'
        send(b'j j s', 'Explicit app opt-out and title opt-in, preserving sibling settings')
        wait('Saved; takes effect now')
        history = saved()['history']
        assert history == dict(enabled=True, paused=True, retention_days=0, exclude_apps='private-editor-q, Bitwarden', record_app=False, record_window_title=True), history
        send(b'kk\r\x15\rs', 'Explicit empty exclusions must not restore password-manager defaults')
        wait('Saved; takes effect now')
        assert saved()['history']['exclude_apps'] == ''
        assert command('config', 'get', '--json')['history.exclude_apps'] == ''
        send(b'k\r\x159\x1b', 'Cancel retention edit')
        send(b's', 'No changed value after cancel')
        wait('Nothing to save')
        assert saved()['history']['retention_days'] == 0
        snapshot('saved')
    finally: close_form(handle, 'first')
    handle = open_form('reopened')
    master, child, screen, raw, drain, wait, send, snapshot = handle
    try:
        reloaded_frame = snapshot('reopened')
        assert 'empty' in reloaded_frame and '0' in reloaded_frame
        assert command('history', 'status', '--json')['paused']
        send(b'j s', 'Resume from reloaded file')
        wait('Saved; takes effect now')
        assert not saved()['history']['paused'] and generation() == 1
        send(b'k s', 'Disable from reloaded file, invalidating old sessions')
        wait('Saved; takes effect now')
        assert not saved()['history']['enabled'] and generation() == 2
        state = command('history', 'status', '--json')
        assert not state['enabled'] and not state['paused']
    finally: close_form(handle, 'reopened')
    assert not select.select([audio], [], [], 0)[0], 'Configurator must never open the private audio sentinel'
    audio.close()
    assert not (root/'data/voxtype/history').exists(), 'Settings-only journey must not create dictation records'
    assert hashlib.sha256(exe.read_bytes()).hexdigest() == native_sha
    result = dict(passed=True, case=case, native_sha256=native_sha, events=events,
                  config=saved(), unselected_sentinel_unchanged=sentinel.read_bytes()==sentinel_before,
                  generation=2, no_audio_connection=True, no_recording_or_inference=True,
                  limits='Actual configure PTY and canonical status, private atomic saves/generations only. Does not certify daemon capture/inference or graphical configurator.')
    Path('/work/out/history-config-'+case+'.json').write_text(json.dumps(result, indent=2)+'\n')
    print('ACTUAL_HISTORY_CONFIGURATOR_RESULT', json.dumps(result), flush=True)
