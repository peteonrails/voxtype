.pragma library

// Who is who in a meeting's colours: which colour slot a speaker has, which
// speaker a stretch of the waveform belongs to, and the colours themselves.
// Plain functions over plain data, so they are tested without a window.
//
// A speaker is known by its key ("You", "Remote 2"), which a rename does not
// change; `labels` (key -> name) is what a rename changes. Two keys given the
// same name are one speaker, as they are one chip.

var YOU = "You"

// The slot for the microphone side, which keeps the theme's accent, and for
// a speaker nothing is known about.
var YOU_SLOT = -1
var NO_SLOT = -2

// `slots` (key -> slot) with `key` in it. A new speaker takes the lowest slot
// no one holds, and keeps it for as long as the meeting is on screen: lines
// moved away, or back, never hand its slot to someone else. Returns `slots`
// itself when nothing changes.
function withSpeaker(slots, key) {
  key = String(key)
  if (key === "" || key === YOU || slots[key] !== undefined) return slots
  var taken = ({})
  var next = ({})
  for (var k in slots) {
    next[k] = slots[k]
    taken[slots[k]] = true
  }
  var slot = 0
  while (taken[slot]) slot++
  next[key] = slot
  return next
}

// The daemon gave a speaker a new key (`speaker_renamed`): the slot goes with
// it, unless the new key already has its own.
function renamedSlots(slots, from, to) {
  from = String(from)
  to = String(to)
  if (from === to || slots[from] === undefined) return slots
  var next = ({})
  for (var k in slots) if (k !== from) next[k] = slots[k]
  if (to !== YOU && next[to] === undefined) next[to] = slots[from]
  return next
}

function nameOf(labels, key) {
  var name = labels ? labels[key] : undefined
  return name !== undefined && name !== "" ? String(name) : String(key)
}

function fold(labels, key) { return nameOf(labels, key).toLowerCase() }

// The slot a speaker is drawn in: its own, or the lowest among the keys that
// share its name. You, and anyone named as you are, is YOU_SLOT.
function colorSlot(slots, labels, key) {
  key = String(key)
  if (key === YOU) return YOU_SLOT
  var name = fold(labels, key)
  if (name === fold(labels, YOU)) return YOU_SLOT
  var best = slots[key] !== undefined ? slots[key] : NO_SLOT
  for (var k in slots)
    if (k !== key && fold(labels, k) === name && (best === NO_SLOT || slots[k] < best)) best = slots[k]
  return best
}

// Which side of the recording a line is on: "mic" (you) or "call". Its
// source says; a line without one goes by its speaker.
function sideOf(line) {
  if (line.source === "microphone") return "mic"
  if (line.source === "loopback") return "call"
  return line.speaker === YOU ? "mic" : "call"
}

// The speaker of each of `count` buckets, `bucketMs` long: the one whose
// lines on `side` ("mic", "call", or "all" for a single mixed track) cover
// the most of the bucket's time, however little that is. Two who cover the
// same share: the one who started first. "" where no line reaches. Speakers
// who share a name count as one, and the key returned is the first of them
// heard in that bucket.
function bucketSpeakers(lines, count, bucketMs, side, labels) {
  var out = []
  var tally = []
  for (var i = 0; i < count; i++) {
    out.push("")
    tally.push(null)
  }
  if (!(bucketMs > 0)) return out
  for (var n = 0; n < lines.length; n++) {
    var line = lines[n]
    if (side !== "all" && sideOf(line) !== side) continue
    var start = Number(line.startMs) || 0
    // A line with no length still marks where it was said.
    var end = Math.max(Number(line.endMs) || 0, start + 1)
    var name = fold(labels, line.speaker)
    var last = Math.min(count - 1, Math.floor((end - 1) / bucketMs))
    for (var b = Math.max(0, Math.floor(start / bucketMs)); b <= last; b++) {
      var covered = Math.min(end, (b + 1) * bucketMs) - Math.max(start, b * bucketMs)
      if (covered <= 0) continue
      if (tally[b] === null) tally[b] = ({})
      var t = tally[b][name]
      if (t === undefined) tally[b][name] = { key: String(line.speaker), ms: covered, from: start }
      else {
        t.ms += covered
        if (start < t.from) { t.from = start; t.key = String(line.speaker) }
      }
    }
  }
  for (var j = 0; j < count; j++) {
    var best = null
    for (var who in tally[j]) {
      var c = tally[j][who]
      if (best === null || c.ms > best.ms || (c.ms === best.ms && c.from < best.from)) best = c
    }
    if (best !== null) out[j] = best.key
  }
  return out
}

// ------------------------------------------------------------ colours

// A colour as [r, g, b] in 0..1, from a QML color or "#rrggbb".
function rgbOf(c) {
  if (typeof c === "string") {
    var m = /^#?([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/i.exec(c)
    return m ? [parseInt(m[1], 16) / 255, parseInt(m[2], 16) / 255, parseInt(m[3], 16) / 255] : [0, 0, 0]
  }
  return [Number(c.r), Number(c.g), Number(c.b)]
}

function toLinear(v) { return v <= 0.04045 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4) }
function fromLinear(v) { return v <= 0.0031308 ? v * 12.92 : 1.055 * Math.pow(v, 1 / 2.4) - 0.055 }

// OKLab: equal steps in it look like equal steps, which HSL's do not (its
// yellow is far lighter than its blue at the same "lightness").
function toOklab(rgb) {
  var r = toLinear(rgb[0]), g = toLinear(rgb[1]), b = toLinear(rgb[2])
  var l = Math.cbrt(0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b)
  var m = Math.cbrt(0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b)
  var s = Math.cbrt(0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b)
  return [0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s,
          1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s,
          0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s]
}

function fromOklab(lab) {
  var l = Math.pow(lab[0] + 0.3963377774 * lab[1] + 0.2158037573 * lab[2], 3)
  var m = Math.pow(lab[0] - 0.1055613458 * lab[1] - 0.0638541728 * lab[2], 3)
  var s = Math.pow(lab[0] - 0.0894841775 * lab[1] - 1.2914855480 * lab[2], 3)
  return [4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
          -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
          -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s]
}

// The sRGB colour at a lightness, chroma and hue (degrees), with the chroma
// brought down until a screen can show it.
function fromLch(lightness, chroma, hue) {
  var h = hue * Math.PI / 180
  for (var c = chroma; ; c *= 0.94) {
    var lin = fromOklab([lightness, c * Math.cos(h), c * Math.sin(h)])
    var ok = c < 0.005
    if (!ok) ok = lin[0] >= -0.0005 && lin[0] <= 1.0005 && lin[1] >= -0.0005 && lin[1] <= 1.0005
      && lin[2] >= -0.0005 && lin[2] <= 1.0005
    if (ok) return [fromLinear(Math.max(0, Math.min(1, lin[0]))), fromLinear(Math.max(0, Math.min(1, lin[1]))),
                    fromLinear(Math.max(0, Math.min(1, lin[2])))]
  }
}

// The speakers' places round the colour wheel, in the order they are handed
// out: nine places with yours (the accent) as place 0, so that the first
// speakers are the furthest from you and from each other.
var HUE_PLACES = [4, 7, 2, 5, 1, 6, 3, 8]

// The speakers' colours for a theme, as [r, g, b] each: eight hues round the
// wheel from the accent's (which is yours), light on a dark background and
// dark on a light one so a name in its colour reads as text. Hues next to
// each other, and next to the accent, also differ in lightness, for eyes
// that tell hues apart poorly. `count` beyond eight gives eight: speakers
// past that share.
function palette(accent, bg, fg, count) {
  var a = toOklab(rgbOf(accent))
  var dark = toOklab(rgbOf(bg))[0] < toOklab(rgbOf(fg))[0]
  // A grey accent has no hue to keep away from.
  var base = Math.sqrt(a[1] * a[1] + a[2] * a[2]) < 0.03 ? 250 : Math.atan2(a[2], a[1]) * 180 / Math.PI
  var tiers = dark ? [0.84, 0.76, 0.68] : [0.42, 0.50, 0.58]
  // The accent counts as the lightness nearest its own, so its neighbours
  // get the other two.
  var mine = 0
  for (var t = 1; t < tiers.length; t++) if (Math.abs(tiers[t] - a[0]) < Math.abs(tiers[mine] - a[0])) mine = t
  var out = []
  for (var i = 0; i < Math.min(count, HUE_PLACES.length); i++) {
    var place = HUE_PLACES[i]
    // 50 degrees clear of the accent on either side, the rest shared out.
    out.push(fromLch(tiers[(mine + place) % 3], dark ? 0.13 : 0.14, base + 50 + (place - 1) * 260 / 7))
  }
  return out
}
