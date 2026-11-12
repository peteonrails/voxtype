#!/bin/bash
# Meeting echo cancellation, end to end, against a private PipeWire with
# null sinks only. Run it inside an isolated session (bwrap with its own
# /dev and XDG_RUNTIME_DIR): it starts its own pipewire and must never meet
# the user's sound server or real speakers.
#
#   tests/sandbox/meeting-aec.sh VOXTYPE_BIN YOU.wav REMOTE.wav
#
# YOU.wav and REMOTE.wav are two sides of a conversation (16 kHz mono, the
# remote side speaking while you are silent). "speakers" is the default sink
# and leaks into the mic at 0.12 gain with 30 ms of latency, as a laptop
# speaker does, and the mic carries a quiet room's hiss (-65 dBFS) as a
# real one does. Each mode records an echo-only meeting and a call:
#   - webrtc: the remote sentences must not reach the mic track, and the
#     echo energy on the kept mic audio must drop by at least 20 dB against
#     disabled; your own sentences must all still be there, at about the
#     level the control records them (a silent mic track would pass the
#     echo checks)
#   - disabled: the control, where they do
# then a call played on a sink other than the default, named by
# loopback_device, must be cancelled against that sink; a canceller
# that hears nothing (its capture pointed at a silent source) must be left
# for the direct mic within seconds; and a meeting on the canceller is
# killed with SIGKILL, and the canceller must be gone. The loopback track is
# off except in the call-sink case, which is judged on the kept audio, so
# transcript dedup (which also strips exact echoes) cannot hide the echo.
set -u
BIN=$1; YOU=$2; REMOTE=$3
[ -e /dev/snd ] && { echo "refusing: /dev/snd is visible, run this in an isolated session"; exit 2; }
OUT=${VOXTYPE_TEST_OUT:-$(mktemp -d)}; mkdir -p $OUT; export XDG_RUNTIME_DIR=${XDG_RUNTIME_DIR:-$OUT/run}; mkdir -p $XDG_RUNTIME_DIR
export XDG_CONFIG_HOME=$OUT/config XDG_DATA_HOME=$OUT/data XDG_STATE_HOME=$OUT/state
mkdir -p $XDG_CONFIG_HOME/voxtype $XDG_DATA_HOME/voxtype/models
ln -sf "${VOXTYPE_TEST_MODEL:-$HOME/.local/share/voxtype/models/ggml-base.en.bin}" $XDG_DATA_HOME/voxtype/models/ggml-base.en.bin
declare -A DB SPEECH
FAIL=0; fail() { echo "FAIL: $*"; FAIL=1; }

pipewire >$OUT/pw.log 2>&1 & sleep 1
wireplumber >$OUT/wp.log 2>&1 & pipewire-pulse >$OUT/pwp.log 2>&1 & sleep 2
pactl load-module module-null-sink sink_name=speakers >/dev/null
pactl load-module module-null-sink sink_name=micsink >/dev/null
pactl load-module module-remap-source master=micsink.monitor source_name=mic >/dev/null
pactl set-default-sink speakers; pactl set-default-source mic
# A real mic is never digitally silent: a quiet room's hiss, looped.
python3 -c "
import wave, random, struct
random.seed(7); a = 10 ** (-65 / 20) * 3 ** 0.5
w = wave.open('$OUT/hiss.wav', 'wb'); w.setnchannels(1); w.setsampwidth(2); w.setframerate(16000)
w.writeframes(b''.join(struct.pack('<h', round(random.uniform(-a, a) * 32768)) for _ in range(16000 * 20)))"
( while :; do paplay -d micsink $OUT/hiss.wav; done ) & HISS=$!
ECHO=$(pactl load-module module-loopback source=speakers.monitor sink=micsink latency_msec=30)
sleep 1
SI=$(pactl -f json list sink-inputs | python3 -c "import json,sys; print(next(i['index'] for i in json.load(sys.stdin) if str(i.get('owner_module'))=='$ECHO'))")
pactl set-sink-input-volume $SI 0.12

daemon() { # $1 = echo_cancel, $2 = mic_device (default "default"), $3 = loopback_device (default "disabled")
  cat > $XDG_CONFIG_HOME/voxtype/config.toml <<CFG
engine = "whisper"
[whisper]
model = "base.en"
[hotkey]
enabled = false
[osd]
enabled = false
[meeting]
enabled = true
chunk_duration_secs = 10
retain_audio = true
[meeting.audio]
echo_cancel = "$1"
mic_device = "${2:-default}"
# No loopback track: transcript dedup strips mic lines the loopback also
# holds, and with this test's perfect-copy echo it would hide the echo
# from the control. Without it the test measures the canceller alone.
loopback_device = "${3:-disabled}"
[meeting.summary]
auto = false
CFG
  $BIN -v daemon >>$OUT/daemon.log 2>&1 & DPID=$!
  for i in $(seq 1 60); do [ -S $XDG_RUNTIME_DIR/voxtype/meeting.sock ] && return; sleep 1; done
}
state() { for i in $(seq 1 120); do grep -q "$1" $XDG_RUNTIME_DIR/voxtype/meeting_state 2>/dev/null && return; sleep 0.5; done; }
meeting() { # $1 = title, $2 = "call" or "echo"
  $BIN meeting start --title "$1" >/dev/null; state recording; sleep 1
  STATUS=$($BIN meeting status --json)
  HELLO=$(timeout 2 socat -u UNIX-CONNECT:$XDG_RUNTIME_DIR/voxtype/meeting.sock - 2>/dev/null | head -1)
  paplay -d ${CALL_SINK:-speakers} "$REMOTE" & local p=$!
  [ "$2" = call ] && paplay -d micsink "$YOU"
  wait $p; sleep 2
  $BIN meeting stop >/dev/null; state idle; sleep 1
}
mic_text() { $BIN meeting show latest --json | python3 -c "
import json,sys; print(' '.join(s['text'] for s in json.load(sys.stdin)['transcript']['segments'] if s['source']=='microphone'))"; }
folder() { $BIN meeting show latest --json | python3 -c "import json,sys; print(json.load(sys.stdin)['metadata']['storage_path'])"; }
mic_db() { ffmpeg -v error -i "$(folder)/mic.opus" -f s16le -ac 1 -ar 16000 - | python3 -c "
import sys, math, array
a = array.array('h', sys.stdin.buffer.read())
e = sum(x*x for x in a) / max(1, len(a)) / 32768**2
print('%.1f' % (10*math.log10(max(e, 1e-20))))"; }
# The level of your speech on the kept mic track: the loudest fifth of its
# 30 ms frames, which in a call are yours.
speech_db() { ffmpeg -v error -i "$(folder)/mic.opus" -f s16le -ac 1 -ar 16000 - | python3 -c "
import sys, math, array
a = array.array('h', sys.stdin.buffer.read())
f = sorted(sum(x*x for x in a[i:i+480]) / 480 / 32768**2 for i in range(0, len(a) - 480, 480))
top = f[-max(1, len(f) // 5):]
print('%.1f' % (10*math.log10(max(sum(top) / len(top), 1e-20))))"; }
# How many of the words only you say are on the mic track, in percent.
own_words() { python3 -c "
import re,sys
you=set(re.findall(r'[a-z]+', sys.argv[1].lower())); remote=set(re.findall(r'[a-z]+', sys.argv[2].lower()))
mine=set(re.findall(r'[a-z]+', sys.argv[3].lower())); only=you-remote
print(round(100*len(only & mine)/max(1,len(only))))" "$YOU_TEXT" "$REMOTE_TEXT" "$1"; }
# Words only the remote side says.
REMOTE_TEXT=$($BIN -q transcribe "$REMOTE" 2>/dev/null | tail -1)
remote_words() { python3 -c "
import re,sys
remote=set(re.findall(r'[a-z]+', sys.argv[1].lower())); mine=set(re.findall(r'[a-z]+', sys.argv[2].lower()))
you=set(re.findall(r'[a-z]+', sys.argv[3].lower()))
print(len((remote - you) & mine))" "$REMOTE_TEXT" "$1" "$YOU_TEXT"; }
YOU_TEXT=$($BIN -q transcribe "$YOU" 2>/dev/null | tail -1)

for mode in webrtc disabled; do
  daemon $mode
  meeting "echo $mode" echo; DB[$mode]=$(mic_db 2>/dev/null || echo 0)
  meeting "call $mode" call
  M=$(python3 -c "import json; print(json.load(open('$(folder)/meeting.json'))['echo_cancel'])")
  S=$(echo "$STATUS" | python3 -c "import json,sys; print(json.load(sys.stdin)['echo_cancel'])")
  H=$(echo "$HELLO" | python3 -c "import json,sys; print(json.load(sys.stdin).get('echo_cancel'))")
  TEXT=$(mic_text); LEAK=$(remote_words "$TEXT"); OWN=$(own_words "$TEXT"); SPEECH[$mode]=$(speech_db)
  echo "$mode: meeting.json echo_cancel=$M, status echo_cancel=$S, hello echo_cancel=$H, echo-only mic ${DB[$mode]} dBFS, remote-only words on the mic track: $LEAK, your words kept: $OWN%, your speech at ${SPEECH[$mode]} dBFS"
  echo "  mic track: $TEXT"
  [ "$OWN" -ge 80 ] || fail "$mode: only $OWN% of your words reached the mic track"
  want=$([ $mode = webrtc ] && echo webrtc || echo none)
  [ "$M" = "$want" ] || fail "$mode: meeting.json says $M"
  [ "$S" = "$want" ] || fail "$mode: meeting status --json says $S"
  [ "$H" = "$want" ] || fail "$mode: meeting.sock hello says $H"
  [ $mode = webrtc ] && [ "$LEAK" -gt 1 ] && fail "webrtc: $LEAK remote words reached the mic track"
  [ $mode = disabled ] && [ "$LEAK" -lt 5 ] && fail "disabled: the control shows no echo ($LEAK words); the test proves nothing"
  kill $DPID; wait $DPID 2>/dev/null
done
python3 -c "import sys; d=float(sys.argv[2])-float(sys.argv[1]); print('echo reduction on the kept mic track: %.1f dB' % d); sys.exit(0 if d >= 20 else 1)" "${DB[webrtc]}" "${DB[disabled]}" || fail "echo reduction under 20 dB"
python3 -c "import sys; d=float(sys.argv[2])-float(sys.argv[1]); print('your speech through the canceller: %+.1f dB against the control' % -d); sys.exit(0 if abs(d) <= 6 else 1)" "${SPEECH[webrtc]}" "${SPEECH[disabled]}" || fail "your speech level through the canceller is off by more than 6 dB"

# Two cases where auto must leave the canceller off, saying why. (The
# unmapped mic is not a real device either, so that meeting may not start
# at all; what counts is that the canceller never did.)
skipped() { # $1 = what, $2 = expected log reason
  $BIN meeting start --title "$1" >/dev/null; sleep 4
  local up=$(pactl list short sources | grep -c voxtype_meeting_aec_source)
  $BIN meeting stop >/dev/null 2>&1; state idle
  [ "$up" = 0 ] || fail "$1: the canceller ran"
  grep -q "$2" $OUT/daemon.log || fail "$1: no log line saying '$2'"
  echo "$1: no canceller, log says '$2'"
}
daemon auto "hw:CARD=Nope,DEV=0"
skipped "unmapped mic_device" "is not a PipeWire source"
kill $DPID; wait $DPID 2>/dev/null
SYS=$(pactl load-module module-echo-cancel aec_method=webrtc source_master=mic sink_master=speakers source_name=users_own_aec sink_name=users_own_aec_sink)
pactl set-default-source users_own_aec
daemon auto
skipped "a system echo canceller as default source" "already an echo canceller"
kill $DPID; wait $DPID 2>/dev/null
pactl set-default-source mic; pactl unload-module $SYS

# A filter as the default source (as Camera Effects is on Pete's machine):
# the canceller must record the mic under it, and still remove the echo.
FX=$(pactl load-module module-remap-source master=mic source_name=fx_mic)
pactl set-default-source fx_mic
daemon webrtc
meeting "filter webrtc" call
grep -q "is a filter over mic" $OUT/daemon.log || fail "the filter default source was not unwrapped to mic"
TEXT=$(mic_text); LEAK=$(remote_words "$TEXT")
echo "filter default source: unwrapped to mic, remote-only words on the mic track: $LEAK"
[ "$LEAK" -gt 1 ] && fail "filter: $LEAK remote words reached the mic track"
kill $DPID; wait $DPID 2>/dev/null
pactl set-default-source mic; pactl unload-module $FX

# The call on a sink other than the default, with loopback_device naming its
# monitor: the canceller must remove that sink's sound, not the default's.
pactl load-module module-null-sink sink_name=callsink >/dev/null
CE=$(pactl load-module module-loopback source=callsink.monitor sink=micsink latency_msec=30); sleep 1
CSI=$(pactl -f json list sink-inputs | python3 -c "import json,sys; print(next(i['index'] for i in json.load(sys.stdin) if str(i.get('owner_module'))=='$CE'))")
pactl set-sink-input-volume $CSI 0.12
daemon webrtc default callsink.monitor
CALL_SINK=callsink meeting "call sink echo" echo; CALL_DB=$(mic_db 2>/dev/null || echo 0)
CALL_SINK=callsink meeting "call sink call" call
TEXT=$(mic_text); OWN=$(own_words "$TEXT")
echo "call on callsink: echo-only mic $CALL_DB dBFS (control ${DB[disabled]}), your words kept: $OWN%"
grep -q "cancelling callsink" $OUT/daemon.log || fail "call sink: the canceller did not take callsink as its reference"
python3 -c "import sys; sys.exit(0 if float(sys.argv[2]) - float(sys.argv[1]) >= 20 else 1)" "$CALL_DB" "${DB[disabled]}" \
  || fail "call sink: the echo of the call's sink was not cancelled"
[ "$OWN" -ge 80 ] || fail "call sink: only $OWN% of your words reached the mic track"
kill $DPID; wait $DPID 2>/dev/null
pactl unload-module $CE

# A canceller that hears nothing: a pipewire on the daemon's PATH points
# its capture at a source nobody plays into. Within seconds the meeting
# must leave it for the mic itself, say so, and keep your words.
pactl load-module module-null-sink sink_name=deadsink >/dev/null
pactl load-module module-remap-source master=deadsink.monitor source_name=deadmic >/dev/null
mkdir -p $OUT/shim; cat > $OUT/shim/pipewire <<'SHIM'
#!/bin/sh
[ "$1" = -c ] || exec /usr/bin/pipewire "$@"
sed -e 's/ target.object = "[^"]*"//' -e 's/node.passive = true/node.passive = true target.object = "deadmic"/' "$2" > "${2%.conf}-dead.conf"
exec /usr/bin/pipewire -c "${2%.conf}-dead.conf"
SHIM
chmod +x $OUT/shim/pipewire
PATH=$OUT/shim:$PATH daemon webrtc
$BIN meeting start --title "deaf canceller" >/dev/null; state recording; sleep 5
paplay -d micsink "$YOU"; sleep 2
UP=$(pactl list short sources | grep -c voxtype_meeting_aec_source)
S=$($BIN meeting status --json | python3 -c "import json,sys; print(json.load(sys.stdin)['echo_cancel'])")
$BIN meeting stop >/dev/null; state idle; sleep 1
M=$(python3 -c "import json; print(json.load(open('$(folder)/meeting.json'))['echo_cancel'])")
TEXT=$(mic_text); OWN=$(own_words "$TEXT")
echo "deaf canceller: status echo_cancel=$S, meeting.json echo_cancel=$M, canceller sources left: $UP, your words kept: $OWN%"
grep -q "echo canceller's mic has been silent" $OUT/daemon.log || fail "deaf canceller: no warning in the log"
case "$S" in gtcrn|none) ;; *) fail "deaf canceller: status says $S";; esac
[ "$M" = "$S" ] || fail "deaf canceller: meeting.json says $M, status $S"
[ "$UP" = 0 ] || fail "deaf canceller: the canceller is still running"
[ "$OWN" -ge 80 ] || fail "deaf canceller: only $OWN% of your words reached the mic track"
kill $DPID; wait $DPID 2>/dev/null

echo "crash: kill -9 the daemon while a meeting runs on the canceller"
daemon webrtc
$BIN meeting start --title crash >/dev/null; state recording; sleep 1
pactl list short sources | grep -q voxtype_meeting_aec_source || fail "the canceller's source is not up"
kill -9 $DPID; sleep 2
pactl list short sources | grep -q voxtype_meeting_aec_source && fail "the canceller outlived the daemon"
pgrep -f "[m]eeting-aec.conf" >/dev/null && fail "a canceller process is still running"
echo "  after kill -9: source gone, no canceller process"

kill $HISS %1 %2 %3 2>/dev/null
[ $FAIL = 0 ] && echo PASS || { echo "logs in $OUT"; exit 1; }
