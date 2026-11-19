.pragma library

// Which host shows the meeting view when more than one is running.
//
// On Omarchy two hosts can run the same view: the Omarchy shell's Voxtype
// plugin and voxtype's standalone Quickshell shell (the Quickshell OSD). Both
// see every meeting start on meeting.sock and both read meeting-view.request,
// so without a rule each opens its own view.
//
// The rule: a host that owns the view (the Omarchy plugin) says so in
// $XDG_RUNTIME_DIR/voxtype/meeting-view.owner for as long as it runs,
//   {"host": "omarchy", "pid": <shell pid>, "updated_at": <unix seconds>}
// rewritten every HEARTBEAT_MS. Any other host stands by while that claim is
// live: it opens nothing itself and leaves requests to the owner. A claim
// older than STALE_SECS is a host that stopped (crashed, quit, or the plugin
// was removed), and the other host takes over again.

var HEARTBEAT_MS = 3000
var STALE_SECS = 10

// The claim in the owner file, or null when there is none worth reading.
function parseClaim(text) {
  var claim = null
  try { claim = JSON.parse(String(text || "")) } catch (e) { return null }
  if (!claim || typeof claim !== "object") return null
  var host = typeof claim.host === "string" ? claim.host : ""
  var pid = Number(claim.pid)
  var at = Number(claim.updated_at)
  if (host === "" || !(pid > 0) || !isFinite(at)) return null
  return { host: host, pid: pid, updated_at: at }
}

// A claim written recently enough that its host is still running. A clock
// that jumped back is allowed the same slack forward.
function claimLive(claim, nowSecs) {
  if (!claim) return false
  var age = nowSecs - claim.updated_at
  return age > -STALE_SECS && age < STALE_SECS
}

// The host `host` stands by for, or "" when it shows the view itself. A host
// that owns the view never stands by, and neither does one reading its own
// claim.
function standbyFor(host, ownsView, claim, nowSecs) {
  if (ownsView || !claimLive(claim, nowSecs) || claim.host === host) return ""
  return claim.host
}

function claimText(host, pid, nowSecs) {
  return JSON.stringify({ host: host, pid: pid, updated_at: Math.floor(nowSecs) }) + "\n"
}

// The meeting-view.request a host writes when it hands an open view to the
// owner: the same meeting, tab and pill state, so the view moves rather than
// closes. `view` is {minimized, meetingId, active, tab}; a live meeting needs
// no id (the owner is following it already).
function handoffRequest(view, nowMs) {
  return {
    meeting_id: !view.active && view.meetingId ? String(view.meetingId) : null,
    ask_id: null,
    tab: view.tab ? String(view.tab) : null,
    ask: null,
    open: true,
    window: view.minimized ? "minimize" : null,
    requested_at: Math.floor(nowMs)
  }
}
