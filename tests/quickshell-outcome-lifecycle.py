#!/usr/bin/env python3
"""Isolated QML outcome regressions; no compositor or microphone access.

Run inside the review Bubblewrap sandbox. Only the PanelWindow wrapper is
adapted to an Item. All tested handlers/content are the actual source.
"""
import pathlib
import re
import shutil
import subprocess
import tempfile

root = pathlib.Path(__file__).resolve().parents[1]
(root / "target").mkdir(exist_ok=True)
with tempfile.TemporaryDirectory(prefix="outcome-", dir=root / "target") as work:
    work = pathlib.Path(work)
    shutil.copytree(root / "quickshell/voxtype-shared", work / "voxtype-shared")
    source = (root / "quickshell/OsdSurface.qml").read_text()
    source = source.replace("import Quickshell.Wayland\n", "").replace("PanelWindow {", "Item {", 1)
    source, count = re.subn(r"    anchors \{\n.*?    mask: Region \{\n.*?\n    \}\n", "", source, count=1, flags=re.S)
    assert count == 1, "platform wrapper changed; update the test adapter explicitly"
    (work / "OffscreenSurface.qml").write_text(source)
    (work / "shell.qml").write_text('''import QtQuick
import Quickshell
ShellRoot {
 id: tests
 property int passed: 0
 property int failed: 0
 Component { id: factory; OffscreenSurface { width:640; height:480 } }
 function check(ok, label) { if(ok) { passed++; console.info("PASS "+label); } else { failed++;console.error("FAIL "+label); } }
 function outcome(id, suppressed, kind) { return { outcome:kind || "ok", chars:42, at_ms:Date.now(), session_started_at_ms:id, suppressed:suppressed }; }
 Timer { interval:100;running:true;onTriggered: {
  let now=Date.now();let s=factory.createObject(tests);
  s.outcome=tests.outcome(now-1000,true);tests.check(!s.flashing,"cold restart suppressed result stays quiet");s.destroy();
  s=factory.createObject(tests);s.daemonState="recording";s.session={started_at_ms:now,max_duration_secs:60};
  s.outcome=tests.outcome(now-1000,false,"error");tests.check(!s.flashing,"old result cannot cover newer recording");s.destroy();
  s=factory.createObject(tests);s.daemonState="recording";s.session={started_at_ms:now,max_duration_secs:60};
  s.outcome=tests.outcome(now,false);tests.check(!s.flashing,"matching result waits while recording");
  s.daemonState="idle";tests.check(s.flashing,"matching queued result appears at idle");s.destroy();
  s=factory.createObject(tests);s.outcome={outcome:"ok",at_ms:now,chars:42};tests.check(!s.flashing,"unidentified pre-release outcome is ignored safely");s.destroy();
  s=factory.createObject(tests);s.outcome=tests.outcome(now,false,"cancelled");tests.check(!s.flashing,"cancel stays quiet");s.destroy();
  s=factory.createObject(tests);s.outcome=tests.outcome(now,false,"too_short");tests.check(!s.flashing,"accidental tap stays quiet");s.destroy();
  s=factory.createObject(tests);s.session={started_at_ms:now,max_duration_secs:60};s.session=null;
  s.outcome=tests.outcome(now-1000,false);tests.check(!s.flashing,"older result rejected after latest session metadata clears");s.destroy();
  console.info("OUTCOME_LIFECYCLE_RESULT passed="+passed+" failed="+failed);Qt.exit(failed>0?1:0);
 } }
}
''')
    result = subprocess.run(["timeout", "15s", "qs", "-p", str(work), "--no-color"], capture_output=True, text=True)
    print(result.stdout + result.stderr)
    raise SystemExit(result.returncode)
