import QtQuick
import QtTest
import Quickshell
import Quickshell.Io
import "plugin" as Plugin

// Driven by test-osd-panel.py. Banner buttons get real pointer clicks; a style
// pick goes in through the Style row's own apply(), the path its picker takes.
// The Python side checks every command the fake CLI and fake systemctl saw.
Item {
  id: host
  Plugin.Panel { id: panel; manifest: ({__sourceDir: __PLUGIN__}) }

  // Restarts "outside the panel" are the fake systemctl, run from here.
  Process {
    id: helper
    property bool done: true
    onExited: helper.done = true
  }

  TestCase {
    id: tc
    parent: panel.renderTest
    name: "OsdPanel"
    when: false
    property bool started: false
    property string failure: ""
    property int osdLoads: 0
    property int unitLoads: 0

    Connections {
      target: panel.cliTest
      function onOsdStatusLoaded(status) { tc.osdLoads++ }
      function onUnitLoaded(state, pid) { tc.unitLoads++ }
    }

    readonly property string scenario: __SCENARIO__
    readonly property string startSection: scenario === "threads" ? "Engine"
      : (scenario === "stalebanner" ? "Audio" : "OSD")

    Timer {
      interval: 100
      running: !tc.started
      repeat: true
      onTriggered: {
        if (!panel.opened || panel.keys.length === 0 || tc.osdLoads === 0 || tc.unitLoads === 0) return
        if (panel.activeSection !== tc.startSection) return
        tc.started = true
        try {
          var s = tc.scenario
          if (s === "osdoffer") tc.driveOffer()
          else if (s === "osdusefail") tc.driveUseFail()
          else if (s === "osdnotnow" || s === "osdnotnowfail") tc.driveNotNow()
          else if (s === "osdnoqs" || s === "osddismissed") tc.driveNoOffer()
          else if (s === "osdold") tc.driveOld()
          else if (s === "stylequickshell") tc.driveStyleSwitches()
          else if (s === "stylenoqs") tc.driveStyleUnavailable()
          else if (s === "styleactive") tc.driveStyleActive()
          else if (s === "stalebanner") tc.driveStaleBanner()
          else if (s === "threads") tc.driveThreads()
          else tc.check(false, "unknown scenario " + s)
        } catch (e) {
          if (tc.failure === "") tc.failure = String(e && e.message ? e.message : e)
        }
        console.log(tc.failure === "" ? "OSD_PANEL_PASS " + tc.scenario
                                      : "OSD_PANEL_FAIL " + tc.scenario + ": " + tc.failure)
        Qt.callLater(Qt.quit)
      }
    }

    // ------------------------------------------------------------ helpers

    function check(ok, message) {
      if (!ok) {
        tc.failure = message
        throw new Error(message)
      }
    }

    function waitFor(predicate, message, timeout) {
      var until = Date.now() + (timeout || 5000)
      while (Date.now() < until) {
        if (predicate()) return
        wait(50)
      }
      check(false, "timed out: " + message)
    }

    function find(item, name) {
      if (!item) return null
      if (item.objectName === name) return item
      var kids = item.children || []
      for (var i = 0; i < kids.length; i++) {
        var hit = find(kids[i], name)
        if (hit) return hit
      }
      return null
    }

    function shown(item) {
      if (!item) return false
      for (var p = item; p; p = p.parent) if (!p.visible) return false
      return true
    }

    function click(item, what) {
      check(item !== null, what + " missing")
      check(shown(item), what + " not visible")
      check(item.enabled, what + " disabled")
      // Columns position their children on the next polish.
      wait(100)
      check(item.enabled && shown(item), what + " changed while settling")
      mouseClick(item, item.width / 2, item.height / 2)
    }

    function sh(args) {
      helper.done = false
      helper.command = args
      helper.running = true
      waitFor(function() { return helper.done }, "helper " + args.join(" "))
    }

    // The SettingDelegate on screen for a key.
    function row(key) {
      var hit = null
      function walk(item) {
        if (hit || !item) return
        if (item.specKey !== undefined && item.spec !== undefined && String(item.specKey) === key && shown(item)) {
          hit = item
          return
        }
        var kids = item.children || []
        for (var i = 0; i < kids.length; i++) walk(kids[i])
      }
      walk(panel.formTest)
      return hit
    }

    function rowText(key, name) {
      var r = row(key)
      var t = r ? find(r, name) : null
      return t && shown(t) ? String(t.text) : ""
    }

    function offerBanner() { return find(panel.renderTest, "osdOfferBanner") }
    function offerShown() { return shown(offerBanner()) }

    function goTo(section) {
      panel.activeSection = section
      wait(150)
    }

    function styleHelp() { return rowText("osd.style", "settingHelpNote") }
    function styleNotice() { return rowText("osd.style", "settingNotice") }

    function pickStyle(name) {
      var r = row("osd.style")
      check(r !== null, "Style row missing")
      r.apply(name)
    }

    // ------------------------------------------------------------ offer

    function driveOffer() {
      waitFor(function() { return tc.offerShown() }, "offer shown on the OSD section")
      var text = find(offerBanner(), "osdOfferText").text
      check(text.indexOf("GTK4") !== -1, "offer names the configured frontend: " + text)
      goTo("Audio")
      check(!offerShown(), "offer not shown on Audio")
      goTo("Engine")
      check(offerShown(), "offer shown on Engine, where model upgrades are offered")
      goTo("OSD")
      check(offerShown(), "offer shown again on OSD")

      var loads = tc.osdLoads
      click(find(offerBanner(), "osdOfferSwitch"), "Switch")
      waitFor(function() { return !tc.offerShown() }, "offer gone after Switch", 8000)
      waitFor(function() { return tc.osdLoads > loads }, "OSD status re-read")
      waitFor(function() { return !panel.cliTest.restarting && !panel.cliTest.osdSwitching }, "restart finished", 8000)
      check(panel.restartError === "", "restart error: " + panel.restartError)
      check(!panel.restartNeeded, "no restart banner after a restart the panel ran")
      waitFor(function() { var r = tc.row("osd.frontend"); return r && r.currentText === "auto" },
              "Frontend row shows auto")
      waitFor(function() { return tc.styleHelp().indexOf("which is the OSD in use") !== -1 },
              "Style help after the switch: " + tc.styleHelp())
    }

    function driveUseFail() {
      waitFor(function() { return tc.offerShown() }, "offer shown")
      click(find(offerBanner(), "osdOfferSwitch"), "Switch")
      waitFor(function() {
        var e = tc.find(tc.offerBanner(), "osdOfferError")
        return tc.shown(e) && String(e.text).indexOf("could not write osd.frontend") !== -1
      }, "use-auto error shown in the offer")
      check(offerShown(), "offer stays when the switch failed")
      wait(500)
      check(!panel.cliTest.restarting, "no restart after a failed switch")
    }

    function driveNotNow() {
      waitFor(function() { return tc.offerShown() }, "offer shown")
      var loads = tc.osdLoads
      click(find(offerBanner(), "osdOfferNotNow"), "Not now")
      waitFor(function() { return !tc.offerShown() }, "offer gone after Not now")
      waitFor(function() { return tc.osdLoads > loads }, "OSD status re-read after Not now")
      goTo("Engine")
      check(!offerShown(), "offer stays gone on Engine")
    }

    function driveNoOffer() {
      check(panel.osdStatus !== null, "status was read")
      wait(300)
      check(!offerShown(), "no offer on OSD")
      goTo("Engine")
      check(!offerShown(), "no offer on Engine")
    }

    function driveOld() {
      check(panel.osdStatus === null, "older CLI gives no status")
      check(!offerShown(), "no offer from an older CLI")
      check(styleHelp() === "Styles belong to the Quickshell OSD.", "Style help with no status: " + styleHelp())
      pickStyle("pill")
      waitFor(function() { var r = tc.row("osd.style"); return r && r.spec.value === "pill" }, "style written")
      check(styleNotice() === "", "no notice for an older CLI: " + styleNotice())
    }

    // ------------------------------------------------------------ style

    function driveStyleSwitches() {
      check(styleHelp().indexOf("Styles belong to the Quickshell OSD.") === 0, "help names Quickshell: " + styleHelp())
      check(styleHelp().indexOf("the GTK4 OSD") !== -1, "help names the frontend in use: " + styleHelp())
      check(styleHelp().indexOf("also sets the OSD frontend to auto") !== -1, "help says what a pick does")
      pickStyle("pill")
      waitFor(function() { return tc.styleNotice().indexOf("was also set to auto") !== -1 },
              "notice says the frontend switched: " + tc.styleNotice())
      waitFor(function() { var r = tc.row("osd.frontend"); return r && r.currentText === "auto" }, "Frontend row shows auto")
      waitFor(function() { return tc.styleHelp().indexOf("which is the OSD in use") !== -1 },
              "help follows the new frontend: " + tc.styleHelp())
      check(panel.restartNeeded, "restart banner after the writes")
    }

    function driveStyleUnavailable() {
      check(styleHelp().indexOf("not available on this system") !== -1, "help says Quickshell is missing: " + styleHelp())
      pickStyle("pill")
      waitFor(function() { return tc.styleNotice().indexOf("Not changed") !== -1 },
              "notice says nothing changed: " + tc.styleNotice())
      waitFor(function() { var r = tc.row("osd.style"); return r && r.currentText === "hud" },
              "picker back on the configured style: " + JSON.stringify({t: tc.row("osd.style").currentText, v: tc.row("osd.style").spec.value, p: String(tc.row("osd.style").pendingValue)}))
      wait(300)
      check(!panel.restartNeeded, "no restart banner when nothing was written")
    }

    function driveStyleActive() {
      check(styleHelp().indexOf("which is the OSD in use") !== -1, "help: " + styleHelp())
      pickStyle("pill")
      waitFor(function() { var r = tc.row("osd.style"); return r && r.spec.value === "pill" }, "style written")
      wait(300)
      check(styleNotice() === "", "no notice when Quickshell is in use: " + styleNotice())
    }

    // ------------------------------------------------------------ restart banner

    function restartBannerShown() { return shown(find(panel.renderTest, "restartBanner")) }

    function writeStyle(name) {
      goTo("OSD")
      pickStyle(name)
      waitFor(function() { return tc.restartBannerShown() }, "restart banner after a write")
      goTo("Audio")
    }

    function driveStaleBanner() {
      check(panel.mainPid === "1000", "pid before: " + panel.mainPid)
      writeStyle("pill")
      // One full poll with the same daemon must not clear it.
      var loads = tc.unitLoads
      waitFor(function() { return tc.unitLoads >= loads + 2 }, "two unit reads with the same pid", 12000)
      check(restartBannerShown(), "banner stays while the same daemon runs")

      // Restarted outside the panel, with the panel open: the poll sees it.
      sh(["systemctl", "--user", "restart", "voxtype"])
      waitFor(function() { return !tc.restartBannerShown() }, "banner clears on the poll after a new pid", 8000)
      check(panel.mainPid === "1001", "header shows the new pid: " + panel.mainPid)

      // Restarted while the panel was closed (`voxtype setup model`): the
      // reopen's unit read clears it, well before the next poll.
      writeStyle("hud")
      wait(300)
      panel.close()
      sh(["systemctl", "--user", "restart", "voxtype"])
      panel.open('{"section":"Audio"}')
      waitFor(function() { return !tc.restartBannerShown() }, "banner clears on reopen after a new pid", 3000)
      check(panel.mainPid === "1002", "pid after reopen: " + panel.mainPid)
    }

    // ------------------------------------------------------------ threads

    function driveThreads() {
      var r = row("parakeet.threads")
      check(r !== null, "Threads row missing")
      var nf = find(r, "settingNumberField")
      var spin = nf.field
      check(r.unsetNumber, "Threads reads as unset")
      check(spin.displayText !== "1", "unset Threads must not show 1")
      check(spin.displayText === "", "unset Threads shows an empty field: '" + spin.displayText + "'")
      check(shown(find(r, "numberUnsetPlaceholder")), "placeholder shown")
      check(find(r, "numberUnsetPlaceholder").text === "default", "placeholder says default")

      // Focus in and out without typing writes nothing (the Python side checks).
      spin.forceActiveFocus()
      wait(100)
      check(!shown(find(r, "numberUnsetPlaceholder")), "placeholder steps aside while editing")
      panel.renderTest.forceActiveFocus()
      wait(300)
      check(r.currentText === "", "still unset after focus out")

      // Entering a number writes it.
      spin.forceActiveFocus()
      spin.contentItem.text = "8"
      keyClick(Qt.Key_Return)
      waitFor(function() { var x = tc.row("parakeet.threads"); return x && x.spec.value === 8 }, "8 written and re-read")
      r = row("parakeet.threads")
      spin = find(r, "settingNumberField").field
      check(spin.displayText === "8", "shows 8: " + spin.displayText)
      check(!shown(find(r, "numberUnsetPlaceholder")), "no placeholder with a value")

      // Reset to default goes back to unset, honestly.
      panel.renderTest.forceActiveFocus()
      r.unsetRequested("parakeet.threads")
      waitFor(function() { var x = tc.row("parakeet.threads"); return x && x.spec.value === null }, "unset re-read")
      r = row("parakeet.threads")
      spin = find(r, "settingNumberField").field
      check(spin.displayText === "", "empty again after reset: '" + spin.displayText + "'")
      check(shown(find(r, "numberUnsetPlaceholder")), "placeholder back after reset")
    }
  }

  Component.onCompleted: Qt.callLater(function() { panel.open(JSON.stringify({ section: tc.startSection })) })
  Timer { interval: 80000; running: true; onTriggered: { console.log("OSD_PANEL_FAIL timeout"); Qt.quit() } }
}
