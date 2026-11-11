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
