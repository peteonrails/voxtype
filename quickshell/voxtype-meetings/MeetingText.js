.pragma library

// [mm:ss] and [h:mm:ss] citations become links the view follows into the
// transcript: "[1:58]" -> <a href="#t=118000">1:58</a>. Inline HTML rather than
// a markdown link because Text ignores linkColor for markdown, which leaves
// links in a blue that is unreadable on a dark card. A timestamp that is
// already a link's text, "[1:58](...)", is left alone.
function linkify(markdown, color) {
  return String(markdown || "").replace(/\[(\d{1,2}:\d{2}(?::\d{2})?)\](?!\()/g, function(match, stamp) {
    return '<a href="#t=' + stampToMs(stamp) + '" style="color:' + color + '">' + stamp + '</a>'
  })
}

// An answer's citations, as `meeting ask --json` reports them ([{ts, ms}]),
// become links wherever "[ts]" appears in its text. Nothing else is guessed
// at: a bracketed time the agent did not cite stays plain text.
function linkCitations(text, citationsJson, color) {
  var out = String(text || "")
  var list = []
  try { list = JSON.parse(citationsJson || "[]") } catch (e) {}
  for (var i = 0; i < list.length; i++) {
    var c = list[i]
    if (!c || c.ts === undefined) continue
    var stamp = "[" + c.ts + "]"
    var link = '<a href="#t=' + Number(c.ms) + '" style="color:' + color + '">' + c.ts + '</a>'
    out = out.split(stamp).join(link)
  }
  return out
}

// A voxtype command's failure as something a user can act on. `what` names
// the feature ("notes", "the meeting assistant"). A voxtype too old for a
// command says "unrecognized subcommand"; say so plainly.
function cliProblem(stderr, what) {
  var err = String(stderr || "").trim()
  if (/unrecognized subcommand|unexpected argument|unknown config key/i.test(err))
    return "This voxtype is too old for " + what + ". Update voxtype (1.3.0 or newer), then reopen this view."
  // Progress lines ("Summarizing...") come before the error on stderr:
  // the error starts at its "Error:" line when there is one.
  var at = err.search(/^Error:/m)
  if (at > 0) err = err.slice(at)
  return err.replace(/^Error:\s*/, "")
}

function missingCli(bin) {
  return "Could not run " + bin + ". Is voxtype installed and on PATH?"
}

function stampToMs(stamp) {
  var parts = String(stamp).split(":")
  var secs = 0
  for (var i = 0; i < parts.length; i++) secs = secs * 60 + Number(parts[i])
  return secs * 1000
}

// The time a link points at, or -1 if it is not a citation.
function linkMs(link) {
  var m = /^#t=(\d+)$/.exec(String(link))
  return m ? Number(m[1]) : -1
}

function escapeHtml(s) {
  return String(s).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;")
}

// A search snippet as rich text: `plain` with each [start, end) range of
// `ranges` (offsets in code points, as `meeting search` gives them) in bold
// and `color`. Everything else is escaped.
function highlight(plain, ranges, color) {
  var chars = Array.from(String(plain || ""))
  var marks = (ranges || []).slice().sort(function(a, b) { return a[0] - b[0] })
  var out = ""
  var at = 0
  for (var i = 0; i < marks.length; i++) {
    var s = Math.max(at, Math.min(chars.length, Number(marks[i][0])))
    var e = Math.max(s, Math.min(chars.length, Number(marks[i][1])))
    out += escapeHtml(chars.slice(at, s).join(""))
    if (e > s) out += '<b><font color="' + color + '">' + escapeHtml(chars.slice(s, e).join("")) + "</font></b>"
    at = e
  }
  return out + escapeHtml(chars.slice(at).join(""))
}

// "m:ss" or "h:mm:ss" for a time in a meeting.
function clockMs(ms) {
  var s = Math.max(0, Math.floor(Number(ms) / 1000))
  var h = Math.floor(s / 3600)
  var m = Math.floor((s % 3600) / 60)
  var sec = s % 60
  return (h > 0 ? h + ":" + (m < 10 ? "0" : "") : "") + m + ":" + (sec < 10 ? "0" : "") + sec
}
