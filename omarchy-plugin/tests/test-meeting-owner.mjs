#!/usr/bin/env node
// Which host shows the meeting view: the rule in
// quickshell/voxtype-meetings/MeetingOwner.js, checked in plain Node (no Qt).
//
//   node omarchy-plugin/tests/test-meeting-owner.mjs
//
// The file is a QML JavaScript library; its `.pragma library` line is the only
// thing Node cannot read, so it is dropped and the rest runs in a VM context.
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";
import vm from "node:vm";
import assert from "node:assert/strict";

const here = path.dirname(fileURLToPath(import.meta.url));
const source = readFileSync(path.join(here, "../../quickshell/voxtype-meetings/MeetingOwner.js"), "utf8")
  .replace(/^\.pragma library\s*$/m, "");
const MO = {};
vm.runInNewContext(source, MO);

const NOW = 1_800_000_000;
const claim = (host, pid, at) => MO.parseClaim(JSON.stringify({ host, pid, updated_at: at }));

const tests = {
  "a fresh Omarchy claim puts the standalone shell on standby"() {
    assert.equal(MO.standbyFor("standalone", false, claim("omarchy", 42, NOW - 2), NOW), "omarchy");
  },

  "the owner never stands by, even reading another claim"() {
    assert.equal(MO.standbyFor("omarchy", true, claim("omarchy", 42, NOW), NOW), "");
    assert.equal(MO.standbyFor("omarchy", true, claim("gtk4", 7, NOW), NOW), "");
  },

  "a host does not stand by for its own claim"() {
    assert.equal(MO.standbyFor("omarchy", false, claim("omarchy", 42, NOW), NOW), "");
  },

  "a stale claim is a host that stopped: the standalone takes over"() {
    assert.equal(MO.standbyFor("standalone", false, claim("omarchy", 42, NOW - MO.STALE_SECS), NOW), "");
    assert.equal(MO.standbyFor("standalone", false, claim("omarchy", 42, NOW - MO.STALE_SECS + 1), NOW), "omarchy");
  },

  "three heartbeats fit in the stale window"() {
    assert.ok(MO.HEARTBEAT_MS * 3 <= MO.STALE_SECS * 1000, "a single late write must not hand the view over");
  },

  "a clock that jumped back keeps a claim live, but not forever"() {
    assert.equal(MO.standbyFor("standalone", false, claim("omarchy", 42, NOW + 5), NOW), "omarchy");
    assert.equal(MO.standbyFor("standalone", false, claim("omarchy", 42, NOW + MO.STALE_SECS), NOW), "");
  },

  "no claim, or one that cannot be read, means no standby"() {
    assert.equal(MO.standbyFor("standalone", false, null, NOW), "");
    for (const text of ["", "garbage", "{\"host\": \"omarchy\"", "[]", "null", "42",
      JSON.stringify({ host: "", pid: 42, updated_at: NOW }),
      JSON.stringify({ host: "omarchy", pid: 0, updated_at: NOW }),
      JSON.stringify({ host: "omarchy", pid: 42 }),
      JSON.stringify({ host: "omarchy", pid: 42, updated_at: "soon" })]) {
      assert.equal(MO.parseClaim(text), null, text);
      assert.equal(MO.standbyFor("standalone", false, MO.parseClaim(text), NOW), "", text);
    }
  },

  "the claim written is the claim read"() {
    const text = MO.claimText("omarchy", 4242, NOW + 0.9);
    assert.ok(text.endsWith("\n"));
    assert.deepEqual({ ...MO.parseClaim(text) }, { host: "omarchy", pid: 4242, updated_at: NOW });
  },

  "a handoff of a live meeting opens the owner on the same tab and pill state"() {
    const req = MO.handoffRequest({ minimized: true, meetingId: "m1", active: true, tab: "ask" }, NOW * 1000 + 0.5);
    assert.deepEqual({ ...req }, { meeting_id: null, ask_id: null, tab: "ask", ask: null, open: true,
      window: "minimize", requested_at: NOW * 1000 });
  },

  "a handoff of an ended or past meeting names it"() {
    const req = MO.handoffRequest({ minimized: false, meetingId: "m1", active: false, tab: "summary" }, NOW * 1000);
    assert.equal(req.meeting_id, "m1");
    assert.equal(req.window, null);
    assert.equal(req.tab, "summary");
    assert.equal(req.open, true);
  },

  "a handoff of the idle list opens the owner's list"() {
    const req = MO.handoffRequest({ minimized: false, meetingId: "", active: false, tab: "" }, NOW * 1000);
    assert.equal(req.meeting_id, null);
    assert.equal(req.tab, null);
    assert.equal(req.open, true);
  },
};

let failed = 0;
for (const [name, fn] of Object.entries(tests)) {
  try {
    fn();
    console.log("PASS " + name);
  } catch (error) {
    failed++;
    console.log("FAIL " + name + ": " + error.message);
  }
}
console.log(failed ? failed + " failed" : "all " + Object.keys(tests).length + " passed");
process.exit(failed ? 1 : 0);
