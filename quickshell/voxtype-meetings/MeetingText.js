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
