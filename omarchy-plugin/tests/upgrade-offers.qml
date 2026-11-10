import QtQuick
import QtTest
import Quickshell
import "plugin" as Plugin

// Driven by test-upgrade-offers.py, which supplies the plugin path and scenario and
// gives the panel test-only aliases (formTest, cliTest, renderTest). Clicks are
// real pointer events on the rendered banner; the CLI on the other end is
// fake-voxtype-migrate.py.
Item {
  id: host
  Plugin.Panel { id: panel; manifest: ({__sourceDir: __PLUGIN__}) }

  TestCase {
    id: tc
    parent: panel.renderTest
    name: "UpgradeOffers"
    when: false
    property bool started: false
    property string failure: ""
    property int upgradeLoads: 0
    property int linkLoads: 0

    Connections {
      target: panel.cliTest
      function onUpgradeLoaded(status) { tc.upgradeLoads++ }
      function onPluginLinkLoaded(status) { tc.linkLoads++ }
    }

    Timer {
      interval: 100
      running: !tc.started
      repeat: true
      onTriggered: {
        if (!panel.opened || panel.engine !== "parakeet" || panel.activeSection !== "Engine") return
        if (panel.formTest.cardTest.modelRows.length === 0) return
        tc.started = true
        try {
          if (__SCENARIO__ === "main") tc.driveMain()
          else if (__SCENARIO__ === "noflag") tc.driveNoFlag()
          else if (__SCENARIO__ === "nomigrate") tc.driveNoMigrate()
          else if (__SCENARIO__ === "latecancel") tc.driveLateCancel()
          else if (__SCENARIO__ === "linkcopy") tc.driveLinkCopy()
          else if (__SCENARIO__ === "linklinked") tc.driveLinkHidden("linked")
          else if (__SCENARIO__ === "linkunknown") tc.driveLinkHidden("")
          else tc.fail("unknown scenario " + __SCENARIO__)
        } catch (e) {
          if (tc.failure === "") tc.failure = String(e && e.message ? e.message : e)
        }
        console.log(tc.failure === "" ? "UPGRADE_OFFERS_PASS " + __SCENARIO__
                                      : "UPGRADE_OFFERS_FAIL " + __SCENARIO__ + ": " + tc.failure)
        Qt.callLater(Qt.quit)
      }
    }

    function check(ok, message) {
      if (!ok) {
        tc.failure = message
        throw new Error(message)
      }
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
      for (var p = item; p; p = p.parent) if (!p.visible) return false
      return true
    }

    function banner() { return find(panel.renderTest, "upgradeOfferBanner") }
    function el(name) { return find(banner(), name) }
    function click(item, what) {
      check(item !== null, what + " missing")
      check(shown(item), what + " not visible")
      check(item.enabled, what + " disabled")
      // Columns position their children on the next polish. A click sent
      // before it lands where the target used to be after a re-layout.
      wait(100)
      check(item.enabled && shown(item), what + " changed while settling")
      mouseClick(item, item.width / 2, item.height / 2)
    }
    function waitFor(predicate, message, timeout) {
      var until = Date.now() + (timeout || 5000)
      while (Date.now() < until) {
        if (predicate()) return
        wait(50)
      }
      check(false, "timed out: " + message)
    }
    function errorFor(offerId) {
      var offer = el("upgradeOffer:" + offerId)
      return offer ? find(offer, "upgradeOfferError") : null
    }
    function catalogDownloadButtons() {
      var out = []
      function walk(item) {
        if (item.tooltipText !== undefined && String(item.text) === "Download" && shown(item)) out.push(item)
        var kids = item.children || []
        for (var i = 0; i < kids.length; i++) walk(kids[i])
      }
      walk(panel.formTest.cardTest)
      return out
    }

    // License links come from optional catalog fields; a row without a URL
    // (or from an older build without the fields) shows nothing.
    function checkLicenseLinks() {
      var card = panel.formTest.cardTest
      var linked = find(card, "modelLicense:parakeet-tdt-0.6b-v3")
      check(linked && shown(linked), "row with license_url shows a link")
      check(linked.text === "License: CC-BY-4.0", "license link text: " + (linked ? linked.text : ""))
      var noUrl = find(card, "modelLicense:parakeet-tdt-0.6b-v3-Q8_0.gguf")
      check(noUrl && !shown(noUrl), "license without a URL shows no link")
      var older = find(card, "modelLicense:parakeet-ultra-Q8_0.gguf")
      check(older && !shown(older), "row without license fields shows no link")
      check(card.licenseLink("file:///etc/passwd") === "" && card.licenseLink(undefined) === "", "only https links")
      var opened = []
      card.openUrl = function(url) { opened.push(url) }
      wait(100)
      mouseClick(linked, linked.width / 2, linked.height / 2)
      check(JSON.stringify(opened) === JSON.stringify(["https://creativecommons.org/licenses/by/4.0/"]),
            "click opens the license URL: " + JSON.stringify(opened))
    }

    function rowNames() {
      var rows = panel.formTest.cardTest.modelRows
      var out = []
      for (var i = 0; i < rows.length; i++) out.push(rows[i].name)
      return out
    }

    function driveMain() {
      checkLicenseLinks()
      // One-way door: the configured legacy ONNX model stays listed, any
      // other legacy ONNX model is hidden.
      check(rowNames().indexOf("parakeet-tdt-0.6b-v3") !== -1, "configured legacy model listed: " + rowNames())
      check(rowNames().indexOf("parakeet-tdt-0.6b-v2") === -1, "unconfigured legacy model hidden: " + rowNames())
      waitFor(function() { return banner() && shown(banner()) && banner().offers.length === 2 }, "banner with two offers")
      var b = banner()

      // Ultra is listed second by the CLI but recommended, so it renders first.
      check(b.offers[0].id === "parakeet-ultra", "Ultra must be first: " + b.offers[0].id)
      var ultraRow = el("upgradeOffer:parakeet-ultra")
      var formatRow = el("upgradeOffer:format-gguf")
      check(ultraRow && formatRow && ultraRow.y < formatRow.y, "Ultra row must render above the format row")
      check(find(ultraRow, "upgradeOfferTitle").text === "Parakeet Ultra is now available", "Ultra title")
      check(b.formatBytes(740363168) === "740 MB", "download size label")
      check(shown(el("upgradeKeep")), "Keep current model shown before any decline")
      check(shown(el("upgradeKeepHint")), "2.0 hint shown beside Keep for an ONNX model")
      check(el("upgradeKeepHint").text.indexOf("2.0") !== -1, "2.0 hint text: " + el("upgradeKeepHint").text)
      check(!shown(el("upgradeRemovalNotice")), "removal notice hidden before decline")
      check(!shown(el("upgradeCleanup")), "no cleanup offer yet")

      // Keep current model: notice and pin command appear, offers stay.
      click(el("upgradeKeep"), "Keep current model")
      waitFor(function() { return shown(el("upgradeRemovalNotice")) }, "removal notice after decline")
      var message = el("upgradeRemovalMessage")
      var pin = el("upgradePinCommand")
      check(message.text.indexOf("removed in voxtype 2.0") !== -1, "removal message text")
      check(pin.text === "add IgnorePkg = voxtype-bin to /etc/pacman.conf", "pin command text")
      check(message.readOnly && message.selectByMouse && pin.readOnly && pin.selectByMouse, "notice must be selectable, read-only text")
      waitFor(function() { return b.allDeclined }, "re-read marks offers declined")
      check(shown(b), "banner must persist after decline")
      check(!shown(el("upgradeKeep")), "Keep hidden once kept")
      check(b.offers.length === 2, "declined offers still listed")
      // A re-summon resets the local "just declined" flag; the notice must
      // come back from the re-read alone. Wait for that read to land: it
      // rebuilds the offer rows, and a click mid-rebuild hits a dying row.
      var before = tc.upgradeLoads
      panel.open('{"section":"Engine"}')
      waitFor(function() { return tc.upgradeLoads > before }, "re-read after re-summon")
      wait(100)
      check(shown(el("upgradeRemovalNotice")) && b.allDeclined, "notice after re-summon")

      // Accept failure: message under the offer, button becomes Retry, model unchanged.
      click(el("upgradeAccept:parakeet-ultra"), "Update (Ultra)")
      waitFor(function() { var e = errorFor("parakeet-ultra"); return e && shown(e) }, "accept error shown")
      check(errorFor("parakeet-ultra").text.indexOf("sha256 mismatch") !== -1, "error message from the NDJSON error event")
      check(el("upgradeAccept:parakeet-ultra").text === "Retry", "failed offer offers Retry")
      check(el("upgradeAccept:format-gguf").text === "Update", "other offer unaffected")
      check(panel.modelValue === "parakeet-tdt-0.6b-v3", "failure must keep the current model")

      // Retry, then cancel mid-download.
      click(el("upgradeAccept:parakeet-ultra"), "Retry (Ultra)")
      waitFor(function() { return panel.cliTest.upgrading && panel.cliTest.downloadPct === 42 }, "progress reported")
      check(!shown(el("upgradeAccept:parakeet-ultra")), "button replaced by progress while running")
      check(!shown(errorFor("parakeet-ultra")), "retry clears the old error")
      check(!el("upgradeAccept:format-gguf").enabled, "second offer locked during an accept")
      var downloads = catalogDownloadButtons()
      check(downloads.length > 0, "catalog Download buttons present")
      for (var i = 0; i < downloads.length; i++)
        check(!downloads[i].enabled, "catalog Download must go flat during an accept")
      var progress = null
      var kids = el("upgradeOffer:parakeet-ultra")
      ;(function walk(item) {
        if (!progress && item.cancelRequested !== undefined && item.pct !== undefined) progress = item
        var c = item.children || []
        for (var j = 0; j < c.length; j++) walk(c[j])
      })(kids)
      check(progress && shown(progress), "DownloadProgress shown in the offer")
      var cancel = null
      ;(function walk(item) {
        if (!cancel && String(item.tooltipText || "") === "Cancel download") cancel = item
        var c = item.children || []
        for (var j = 0; j < c.length; j++) walk(c[j])
      })(progress)
      click(cancel, "Cancel download")
      waitFor(function() { return !panel.cliTest.upgrading }, "cancel finishes", 8000)
      check(!shown(errorFor("parakeet-ultra")), "cancel shows no error")
      check(el("upgradeAccept:parakeet-ultra").text === "Update", "cancel restores Update")
      check(panel.modelValue === "parakeet-tdt-0.6b-v3", "cancel keeps the current model")

      // Success: offers clear, cleanup offer appears, no false restart banner.
      click(el("upgradeAccept:parakeet-ultra"), "Update (Ultra) again")
      waitFor(function() { return b.offers.length === 0 && b.cleanup !== null }, "offers cleared and cleanup offered", 8000)
      waitFor(function() { return panel.modelValue === "parakeet-ultra-Q8_0.gguf" }, "schema re-read after switch")
      waitFor(function() { return rowNames().indexOf("parakeet-tdt-0.6b-v3") === -1 }, "legacy model hidden after moving to GGUF")
      check(rowNames().indexOf("parakeet-ultra-Q8_0.gguf") !== -1, "GGUF models still listed: " + rowNames())
      check(shown(b), "banner stays for the cleanup offer")
      check(!shown(el("upgradeKeep")) && !shown(el("upgradeRemovalNotice")), "no keep/notice without offers")
      var cleanupText = el("upgradeCleanupText").text
      check(cleanupText.indexOf("parakeet-tdt-0.6b-v3") !== -1 && cleanupText.indexOf("2.6 GB") !== -1, "cleanup text: " + cleanupText)
      check(panel.restartNeeded === false, "accept restarted the daemon itself; no restart banner")

      // Cleanup: with no offers and nothing to clean, the banner goes away.
      click(el("upgradeCleanupButton"), "Remove old model files")
      waitFor(function() { return !shown(banner()) }, "empty offers hide the banner")
      check(panel.upgradeActionError === "", "cleanup error: " + panel.upgradeActionError)

      // A GGUF model offered only Ultra has no removal notice, so Keep has no 2.0 hint.
      var gb = banner()
      gb.status = { schema: 1, current: { engine: "parakeet", model: "parakeet-tdt-0.6b-v3-Q8_0.gguf", format: "gguf" },
        offers: [{ id: "parakeet-ultra", kind: "model", engine: "parakeet", model: "parakeet-ultra-Q8_0.gguf",
          title: "Parakeet Ultra is now available", detail: "", download_bytes: 0, recommended: true, declined: false }],
        removal_notice: null, cleanup: null }
      waitFor(function() { return shown(el("upgradeKeep")) }, "Keep shown for the GGUF-only offer")
      check(!shown(el("upgradeKeepHint")), "no 2.0 hint without a removal notice")
    }

    // A CLI that rejects --progress-format still upgrades: one silent retry
    // without the flag, indeterminate bar, success.
    function driveNoFlag() {
      waitFor(function() { return banner() && shown(banner()) && banner().offers.length === 2 }, "banner with two offers")
      click(el("upgradeAccept:format-gguf"), "Update (format)")
      waitFor(function() { return banner().offers.length === 0 }, "accept via fallback", 8000)
      check(!errorFor("format-gguf") || !shown(errorFor("format-gguf")), "no error after fallback")
    }

    // A cancel after the download finished cannot stop the switch; the
    // panel must report the switch that happened, not a cancellation.
    function driveLateCancel() {
      waitFor(function() { return banner() && shown(banner()) && banner().offers.length === 2 }, "banner with two offers")
      click(el("upgradeAccept:parakeet-ultra"), "Update (Ultra)")
      waitFor(function() { return panel.cliTest.upgrading && panel.cliTest.downloadPct === 100 }, "download finished")
      panel.cliTest.cancelDownload()
      check(panel.cliTest.downloadCancelling, "cancel requested")
      waitFor(function() { return !panel.cliTest.upgrading }, "accept exits", 8000)
      check(!errorFor("parakeet-ultra") || !shown(errorFor("parakeet-ultra")), "no error after a completed switch")
      waitFor(function() { return banner().offers.length === 0 && panel.modelValue === "parakeet-ultra-Q8_0.gguf" }, "switch reflected")
      check(panel.restartNeeded === false, "no restart prompt after the CLI restarted the daemon")
    }

    function copyBanner() { return find(panel.renderTest, "pluginCopyBanner") }

    // A hand-installed copy with a packaged one available: the banner offers the
    // switch, and the switch runs in a terminal after the panel is dismissed.
    function driveLinkCopy() {
      waitFor(function() { return tc.linkLoads > 0 }, "plugin link status read")
      check(panel.pluginLink !== null && panel.pluginLink.state === "copy", "link state copy")
      waitFor(function() { return shown(copyBanner()) }, "copy banner shown")
      var text = find(copyBanner(), "pluginCopyText").text
      check(text === "This is an older copy of Voxtype Settings. Switch to the one that updates with voxtype?",
            "copy banner text: " + text)
      check(shown(panel.formTest.cardTest), "settings still usable beside the banner")
      click(find(copyBanner(), "pluginCopySwitch"), "Switch")
      waitFor(function() { return !panel.opened }, "panel dismissed before the terminal launch")
      // The launch runs on the next event-loop pass; give the fake terminal
      // time to log it before the driver quits.
      wait(800)
    }

    // Linked to the packaged copy, or a CLI that has no `setup omarchy`: no banner.
    function driveLinkHidden(state) {
      waitFor(function() { return tc.linkLoads > 0 }, "plugin link status read")
      wait(200)
      if (state === "") check(panel.pluginLink === null, "unknown command reads as no answer")
      else check(panel.pluginLink !== null && panel.pluginLink.state === state, "link state " + state)
      check(copyBanner() !== null, "copy banner component exists")
      check(!shown(copyBanner()), "no copy banner when " + (state || "the CLI lacks setup omarchy"))
      check(!panel.degraded, "an unknown subcommand must not degrade the panel")
      check(shown(panel.formTest.cardTest), "model card still shown")
    }

    // Older voxtype without `setup migrate`: no banner, the Engine section is intact.
    function driveNoMigrate() {
      wait(1500)
      checkLicenseLinks()
      check(banner() !== null, "banner component exists")
      check(!shown(banner()), "no banner when setup migrate is unavailable")
      check(panel.upgradeStatus === null, "status stays null")
      check(shown(panel.formTest.cardTest), "model card still shown")
    }
  }

  Component.onCompleted: Qt.callLater(function() { panel.open('{"section":"Engine"}') })
  Timer { interval: 40000; running: true; onTriggered: { console.log("UPGRADE_OFFERS_FAIL timeout"); Qt.quit() } }
}
