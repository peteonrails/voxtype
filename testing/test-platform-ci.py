#!/usr/bin/env python3
"""Dependency-light regressions for platform-only CI seams (no Voxtype execution)."""
from pathlib import Path
import re
import subprocess
import unittest

ROOT = Path(__file__).resolve().parents[1]


class PlatformCiTests(unittest.TestCase):
    def test_macos_notification_image_match_covers_every_engine(self):
        engines = (ROOT / "src/config/engines/mod.rs").read_text()
        body = engines.split("pub enum TranscriptionEngine {", 1)[1].split("\n}", 1)[0]
        variants = set(re.findall(r"^    (\w+),$", body, re.M))
        source = (ROOT / "src/notification.rs").read_text()
        match = source.split("let content_image = engine.and_then(|e| match e {", 1)[1].split("\n    });", 1)[0]
        covered = set(re.findall(r"TranscriptionEngine::(\w+)", match))
        self.assertTrue(variants, "engine enum extraction must not be empty")
        self.assertEqual(covered, variants)
        self.assertRegex(match, r"TranscriptionEngine::Gguf\s*=> None")

    def test_tray_bus_has_explicit_self_contained_configuration(self):
        source = (ROOT / "tests/tray_protocol.rs").read_text()
        isolated = source.split("fn isolated(name: &str)", 1)[1].split("\n#[tokio::test]", 1)[0]
        self.assertIn('.arg("--config-file")', isolated)
        self.assertIn('"/tests/fixtures/tray-session.conf"', isolated)
        config = ROOT / "tests/fixtures/tray-session.conf"
        result = subprocess.run([
            "dbus-run-session", "--config-file", str(config), "--",
            "dbus-send", "--session", "--print-reply", "--dest=org.freedesktop.DBus",
            "/org/freedesktop/DBus", "org.freedesktop.DBus.GetId",
        ], capture_output=True, text=True, timeout=10)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertRegex(result.stdout, r'string "[0-9a-f]{32}"')
        self.assertNotIn("<include", config.read_text())


if __name__ == "__main__":
    unittest.main(verbosity=2)
