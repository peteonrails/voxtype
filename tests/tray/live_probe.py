#!/usr/bin/env python3
"""Expose ONLY a recording-free tray probe to the real session bus.
No second daemon, audio/input device, production config, or service control.
"""
import hashlib
import json
import os
from pathlib import Path
import shlex
import subprocess
import time

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / 'tray-evidence/live'
OUT.mkdir(parents=True, exist_ok=True)
BIN = ROOT / 'target-tray/debug/examples/tray_probe'
bus = os.environ['DBUS_SESSION_BUS_ADDRESS']
assert bus.startswith('unix:path='), 'only a filesystem bus socket is supported'
socket = bus[len('unix:path='):].split(',')[0]

def call(*args):
    return subprocess.check_output(['busctl', '--user', *args], text=True).strip()
def registrations():
    raw = call('get-property', 'org.kde.StatusNotifierWatcher', '/StatusNotifierWatcher', 'org.kde.StatusNotifierWatcher', 'RegisteredStatusNotifierItems')
    values = shlex.split(raw)
    assert int(values[1]) == len(values[2:])
    return values[2:]
def prop(service, path, name):
    return call('get-property', service, path, 'org.kde.StatusNotifierItem', name)
def voxtype():
    found = []
    for item in registrations():
        service, suffix = item.split('/', 1)
        path = '/' + suffix
        try:
            if prop(service, path, 'Id') == 's "voxtype"': found.append((service, path))
        except subprocess.CalledProcessError: pass
    return found

before = registrations()
assert not voxtype(), 'a Voxtype tray is already registered; do not create a duplicate'
protected = [Path('/usr/local/bin/voxtype'), Path.home() / '.config/voxtype/config.toml']
hashes = {str(path): hashlib.sha256(path.read_bytes()).hexdigest() for path in protected}
pid_before = subprocess.check_output(['systemctl','--user','show','voxtype','-p','MainPID','--value'], text=True).strip()
args = ['bwrap', '--die-with-parent', '--new-session', '--unshare-all', '--clearenv',
    '--ro-bind', '/usr', '/usr', '--ro-bind', '/etc', '/etc', '--symlink','usr/bin','/bin', '--symlink','usr/lib','/lib', '--symlink','usr/lib','/lib64',
    '--proc','/proc','--dev','/dev','--tmpfs','/tmp','--tmpfs','/run',
    '--ro-bind',str(BIN),'/tray-probe','--ro-bind',socket,'/tray-bus',
    '--setenv','HOME','/tmp','--setenv','PATH','/usr/bin:/bin','--setenv','DBUS_SESSION_BUS_ADDRESS','unix:path=/tray-bus','/tray-probe']
log = (OUT / 'probe.log').open('w')
proc = subprocess.Popen(args, stdin=subprocess.PIPE, stdout=log, stderr=subprocess.STDOUT, text=True)
assert proc.stdin is not None
result = {'before': before, 'binary_sha256': hashlib.sha256(BIN.read_bytes()).hexdigest(), 'protected_before': hashes, 'daemon_pid_before': pid_before, 'states': {}}
try:
    until = time.monotonic() + 6
    while not voxtype() and time.monotonic() < until:
        assert proc.poll() is None, 'probe exited before registration'
        time.sleep(0.1)
    matches = voxtype()
    assert len(matches) == 1, matches
    service, path = matches[0]
    result['service'] = service; result['path'] = path
    for state, status, icon in [('idle','Active','voxtype'),('recording','NeedsAttention','voxtype-recording'),('transcribing','Active','view-refresh-symbolic'),('error','NeedsAttention','dialog-error-symbolic'),('idle','Active','voxtype')]:
        proc.stdin.write(state + '\n'); proc.stdin.flush(); time.sleep(0.5)
        data = {key: prop(service,path,key) for key in ('Id','Title','Status','IconName','ToolTip','Menu')}
        assert data['Status'] == 's "' + status + '"', data
        assert data['IconName'] == 's "' + icon + '"', data
        assert len(voxtype()) == 1
        result['states'][state] = data
        (OUT / ('properties-' + state + '.txt')).write_text('\n'.join(key + ': ' + value for key,value in data.items()) + '\n')
        # Top-right bar only; no unrelated desktop content in the capture.
        if state == 'idle':
            layers = json.loads(subprocess.check_output(['hyprctl', 'layers', '-j'], text=True))
            screensaver = any(item.get('namespace') == 'org.omarchy.screensaver' for output in layers.values() for level in output.get('levels', {}).values() for item in level)
            if screensaver:
                result['visibility_check'] = 'BLOCKED: active org.omarchy.screensaver; did not dismiss it or claim visual tray legibility'
                continue
            monitors = json.loads(subprocess.check_output(['hyprctl','monitors','-j'], text=True))
            monitor = next((m for m in monitors if m.get('focused')), monitors[0])
            width = int(monitor['width'] / monitor['scale'])
            x = monitor['x'] + width - min(width, 900)
            geometry = f"{x},{monitor['y']} {min(width,900)}x60"
            subprocess.run(['grim','-g',geometry,str(OUT / 'omarchy-tray.png')],check=True)
    result['introspection'] = call('introspect',service,path)
    proc.stdin.write('quit\n'); proc.stdin.flush(); proc.wait(timeout=4)
    assert proc.returncode == 0
finally:
    if proc.poll() is None: proc.kill(); proc.wait(timeout=3)
    log.close()
    time.sleep(0.3)
    result['after'] = registrations()
    result['protected_after'] = {str(path): hashlib.sha256(path.read_bytes()).hexdigest() for path in protected}
    result['daemon_pid_after'] = subprocess.check_output(['systemctl','--user','show','voxtype','-p','MainPID','--value'], text=True).strip()
    (OUT / 'manifest.json').write_text(json.dumps(result,indent=2) + '\n')
assert not voxtype(), 'tray registration was not removed'
assert hashes == result['protected_after'], 'production wrapper or config changed'
assert result['daemon_pid_after'] == pid_before, 'production daemon changed during probe'
print('LIVE_OMARCHY_TRAY_PASS: one real item, five state checks, embedded icon/menu, exit removal, production untouched during probe')
