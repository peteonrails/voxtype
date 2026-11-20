#!/usr/bin/env python3
"""Meeting release portability seams; no application or desktop execution."""
from pathlib import Path
import os
import hashlib
import re
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


class MeetingPortabilityTests(unittest.TestCase):
    def test_ci_explicitly_provisions_rust_and_clippy(self):
        workflow = (ROOT / ".github/workflows/build-linux.yml").read_text()
        self.assertIn("uses: dtolnay/rust-toolchain@stable", workflow)
        self.assertIn("components: clippy", workflow)
        self.assertLess(workflow.index("uses: dtolnay/rust-toolchain@stable"),
                        workflow.index("- name: Check release transport and desktop assets"))

    def test_unix_process_groups_have_a_unix_dependency(self):
        manifest = (ROOT / "Cargo.toml").read_text()
        section = re.search(
            r"(?ms)^\[target\.'cfg\(unix\)'\.dependencies\]\n(.*?)(?=^\[|\Z)", manifest
        )
        self.assertIsNotNone(section, "meeting process groups use nix on macOS too")
        assert section is not None
        self.assertRegex(section.group(1), r'(?m)^nix = .*"signal".*"process"')

    def test_recovery_dbus_is_linux_only_with_native_fallback(self):
        source = (ROOT / "src/meeting/recover/notify.rs").read_text()
        self.assertIn('#[cfg(target_os = "linux")]\nuse zbus::', source)
        self.assertIn('#[cfg(target_os = "linux")]\npub async fn announce(', source)
        daemon = (ROOT / "src/daemon.rs").read_text()
        body = daemon.split("fn notify_interrupted_meetings(&self)", 1)[1].split(
            "async fn handle_meeting_triggers", 1
        )[0]
        self.assertIn('#[cfg(target_os = "linux")]\n        tokio::spawn', body)
        self.assertIn('#[cfg(not(target_os = "linux"))]', body)
        self.assertIn('crate::notification::send(&summary, &body).await', body)
        self.assertIn('Run: voxtype meeting recover', body)

    def check_entropy_contract(self, linux):
        source = (ROOT / "src/meeting/actions/oauth.rs").read_text()
        function = source.split("fn random_bytes(", 1)[1].split("\n}\n", 1)[0]
        # Rewrite only the target selector for the extracted function, not the
        # syscalls or their flags. Both branches compile/run in the NAS seam;
        # this is deliberately not claimed as a macOS target build.
        function = function.replace('target_os = "linux"', 'test_linux_rng')
        fixture = (ROOT / "testing/fixtures/oauth-random-contract.rs").read_text()
        program = fixture + "\nfn random_bytes(" + function + "\n}\n"
        with tempfile.TemporaryDirectory(prefix="meeting-random-", dir=os.environ.get("TMPDIR")) as tmp:
            path = Path(tmp)
            (path / "random.rs").write_text(program)
            flags = ["--cfg", "test_linux_rng"] if linux else []
            compile_result = subprocess.run(
                ["rustc", "--edition=2021", "--test", *flags,
                 str(path / "random.rs"), "-o", str(path / "random")],
                capture_output=True, text=True, timeout=300,
            )
            self.assertEqual(compile_result.returncode, 0, compile_result.stderr)
            result = subprocess.run([str(path / "random")], capture_output=True, text=True, timeout=10)
            print("entropy", "linux" if linux else "macos seam",
                  "source_sha256", hashlib.sha256(program.encode()).hexdigest(),
                  "binary_sha256", hashlib.sha256((path / "random").read_bytes()).hexdigest(),
                  "exit", result.returncode, "\n" + result.stdout + result.stderr)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_linux_entropy_waits_for_initialization_and_fails_closed(self):
        self.check_entropy_contract(linux=True)

    def test_macos_entropy_has_no_linux_api_or_weak_fallback(self):
        self.check_entropy_contract(linux=False)

    def test_share_command_condition_is_clippy_clean_and_equivalent(self):
        source = (ROOT / "src/meeting/share.rs").read_text().split("pub fn why(", 1)[1]
        condition = re.search(r"(?m)^        if (.*command.*) \{$", source)
        self.assertIsNotNone(condition)
        assert condition is not None
        program = "fn missing(command: Option<&str>) -> bool { " + condition.group(1) + " }\n" + '''
#[test]
fn command_presence_contract() {
    for (command, expected) in [(None, true), (Some(""), true),
        (Some(" \\t"), true), (Some("sh"), false), (Some(" sh "), false)] {
        assert_eq!(missing(command), expected);
    }
}
'''
        with tempfile.TemporaryDirectory(prefix="meeting-share-", dir=os.environ.get("TMPDIR")) as tmp:
            path = Path(tmp)
            (path / "share.rs").write_text(program)
            result = subprocess.run(
                ["clippy-driver", "--edition=2021", "--test", str(path / "share.rs"),
                 "-o", str(path / "share"), "-D", "clippy::nonminimal_bool", "-D", "warnings"],
                capture_output=True, text=True, timeout=300,
            )
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            result = subprocess.run([str(path / "share")], capture_output=True, text=True, timeout=10)
            print("share condition", "source_sha256", hashlib.sha256(program.encode()).hexdigest(),
                  "binary_sha256", hashlib.sha256((path / "share").read_bytes()).hexdigest(),
                  "exit", result.returncode, "\n" + result.stdout + result.stderr)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main(verbosity=2)
