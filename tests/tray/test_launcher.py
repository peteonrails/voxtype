#!/usr/bin/env python3
"""Exercise the actual canonical launcher, using executable/desktop fixtures."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

LAUNCHER = Path(__file__).resolve().parents[2] / 'packaging/scripts/voxtype-configure-launcher'

class Launcher(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.bin = self.root / 'bin'; self.bin.mkdir()
        self.runtime = self.root / 'run'; self.runtime.mkdir(mode=0o700)
        self.data = self.root / 'data'; (self.data / 'applications').mkdir(parents=True)
        self.log = self.root / 'calls'
        self.env = dict(os.environ, PATH=str(self.bin), HOME=str(self.root), DISPLAY=':99',
            WAYLAND_DISPLAY='', XDG_RUNTIME_DIR=str(self.runtime), XDG_DATA_HOME=str(self.data),
            XDG_DATA_DIRS=str(self.root / 'none'), CALLS=str(self.log), TERMINAL='')
        for tool in ('timeout', 'flock', 'grep', 'awk', 'sed', 'basename'):
            (self.bin / tool).symlink_to('/usr/bin/' + tool)
    def tearDown(self): self.tmp.cleanup()
    def stub(self, name, body='exit 0'):
        p = self.bin / name
        p.write_text('#!/bin/sh\nprintf "%s\\n" "' + name + ' $*" >> "$CALLS"\n' + body + '\n')
        p.chmod(0o755)
    def run_launcher(self):
        return subprocess.run(['/bin/sh', str(LAUNCHER)], env=self.env, capture_output=True, text=True, timeout=5)
    def calls(self): return self.log.read_text() if self.log.exists() else ''
    def test_integrated_quickshell_wins_and_uses_idempotent_open(self):
        self.stub('omarchy-shell', 'printf "ok\\n"')
        self.stub('ghostty')
        r = self.run_launcher()
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(self.calls(), 'omarchy-shell io.voxtype.settings open\n')
    def test_graphical_desktop_gtk_fixture_when_quickshell_unavailable(self):
        self.stub('gtk-settings-fixture')
        self.stub('gio')
        desktop = self.data / 'applications/voxtype-gtk-settings.desktop'
        desktop.write_text('[Desktop Entry]\nType=Application\nName=Voxtype Settings fixture\nCategories=Settings;\nExec=gtk-settings-fixture\nTerminal=false\n')
        self.stub('ghostty')
        r = self.run_launcher()
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn('gio launch ' + str(desktop), self.calls())
        self.assertNotIn('ghostty', self.calls())
    def test_missing_graphical_frontend_falls_back_to_terminal(self):
        self.stub('omarchy-shell', 'printf "error: unknown target\\n"')
        self.stub('ghostty')
        r = self.run_launcher()
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn('ghostty --class=voxtype --command=voxtype configure', self.calls())
    def test_headless_fails_without_spawning(self):
        self.env.update(DISPLAY='', WAYLAND_DISPLAY='')
        self.stub('ghostty')
        self.assertNotEqual(self.run_launcher().returncode, 0)
        self.assertEqual(self.calls(), '')
    def test_no_terminal_or_gui_reports_failure(self):
        self.assertNotEqual(self.run_launcher().returncode, 0)
    def test_only_osd_or_self_desktop_is_not_a_settings_frontend(self):
        (self.data / 'applications/voxtype-configure.desktop').write_text('[Desktop Entry]\nType=Application\nCategories=Settings;\nExec=voxtype-configure-launcher\n')
        (self.data / 'applications/voxtype-osd.desktop').write_text('[Desktop Entry]\nType=Application\nCategories=Settings;\nExec=voxtype-osd-gtk4\n')
        self.stub('gio'); self.stub('ghostty')
        self.assertEqual(self.run_launcher().returncode, 0)
        self.assertNotIn('gio', self.calls())
    def test_graphical_failure_falls_back_once(self):
        self.stub('omarchy-shell', 'exit 1')
        self.stub('ghostty')
        self.assertEqual(self.run_launcher().returncode, 0)
        self.assertEqual(self.calls().count('ghostty '), 1)
    def test_concurrent_requests_do_not_open_multiple_windows(self):
        self.stub('ghostty', '/usr/bin/sleep 0.5')
        processes = [subprocess.Popen(['/bin/sh', str(LAUNCHER)], env=self.env, stdout=subprocess.PIPE, stderr=subprocess.PIPE) for _ in range(4)]
        for proc in processes:
            proc.communicate(timeout=3)
            self.assertEqual(proc.returncode, 0)
        self.assertEqual(self.calls().count('ghostty '), 1)

if __name__ == '__main__': unittest.main(verbosity=2)
