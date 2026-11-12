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
# speaker does. Each mode records an echo-only meeting and a call:
#   - aec: the remote sentences must not reach the mic track, and the echo
#     energy on the kept mic audio must drop by at least 20 dB against off
#   - off: the control, where they do
# then a meeting on the canceller is killed with SIGKILL, and the canceller
# must be gone. The loopback track is off, so transcript dedup (which also
# strips exact echoes) cannot hide the echo from the control.
set -u
BIN=$1; YOU=$2; REMOTE=$3
[ -e /dev/snd ] && { echo "refusing: /dev/snd is visible, run this in an isolated session"; exit 2; }
OUT=$(mktemp -d); export XDG_RUNTIME_DIR=${XDG_RUNTIME_DIR:-$OUT/run}; mkdir -p $XDG_RUNTIME_DIR
export XDG_CONFIG_HOME=$OUT/config XDG_DATA_HOME=$OUT/data XDG_STATE_HOME=$OUT/state
mkdir -p $XDG_CONFIG_HOME/voxtype $XDG_DATA_HOME/voxtype/models
ln -sf "${VOXTYPE_TEST_MODEL:-$HOME/.local/share/voxtype/models/ggml-base.en.bin}" $XDG_DATA_HOME/voxtype/models/ggml-base.en.bin
declare -A DB
FAIL=0; fail() { echo "FAIL: $*"; FAIL=1; }

pipewire >$OUT/pw.log 2>&1 & sleep 1
wireplumber >$OUT/wp.log 2>&1 & pipewire-pulse >$OUT/pwp.log 2>&1 & sleep 2
pactl load-module module-null-sink sink_name=speakers >/dev/null
pactl load-module module-null-sink sink_name=micsink >/dev/null
pactl load-module module-remap-source master=micsink.monitor source_name=mic >/dev/null
pactl set-default-sink speakers; pactl set-default-source mic
ECHO=$(pactl load-module module-loopback source=speakers.monitor sink=micsink latency_msec=30)
sleep 1
SI=$(pactl -f json list sink-inputs | python3 -c "import json,sys; print(next(i['index'] for i in json.load(sys.stdin) if str(i.get('owner_module'))=='$ECHO'))")
pactl set-sink-input-volume $SI 0.12

daemon() { # $1 = echo_cancel
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
# No loopback track: transcript dedup strips mic lines the loopback also
# holds, and with this test's perfect-copy echo it would hide the echo
# from the control. Without it the test measures the canceller alone.
loopback_device = "disabled"
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
  paplay -d speakers "$REMOTE" & local p=$!
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
# Words only the remote side says.
REMOTE_TEXT=$($BIN -q transcribe "$REMOTE" 2>/dev/null | tail -1)
remote_words() { python3 -c "
import re,sys
remote=set(re.findall(r'[a-z]+', sys.argv[1].lower())); mine=set(re.findall(r'[a-z]+', sys.argv[2].lower()))
you=set(re.findall(r'[a-z]+', sys.argv[3].lower()))
print(len((remote - you) & mine))" "$REMOTE_TEXT" "$1" "$YOU_TEXT"; }
YOU_TEXT=$($BIN -q transcribe "$YOU" 2>/dev/null | tail -1)

for mode in aec off; do
  daemon $mode
  meeting "echo $mode" echo; DB[$mode]=$(mic_db 2>/dev/null || echo 0)
  meeting "call $mode" call
  M=$(python3 -c "import json; print(json.load(open('$(folder)/meeting.json'))['echo_cancel'])")
  S=$(echo "$STATUS" | python3 -c "import json,sys; print(json.load(sys.stdin)['echo_cancel'])")
  TEXT=$(mic_text); LEAK=$(remote_words "$TEXT")
  echo "$mode: meeting.json echo_cancel=$M, status echo_cancel=$S, echo-only mic ${DB[$mode]} dBFS, remote-only words on the mic track: $LEAK"
  echo "  mic track: $TEXT"
  [ "$M" = "$mode" ] || fail "$mode: meeting.json says $M"
  [ "$S" = "$mode" ] || fail "$mode: meeting status --json says $S"
  [ $mode = aec ] && [ "$LEAK" -gt 1 ] && fail "aec: $LEAK remote words reached the mic track"
  [ $mode = off ] && [ "$LEAK" -lt 5 ] && fail "off: the control shows no echo ($LEAK words); the test proves nothing"
  kill $DPID; wait $DPID 2>/dev/null
done
python3 -c "import sys; d=float(sys.argv[2])-float(sys.argv[1]); print('echo reduction on the kept mic track: %.1f dB' % d); sys.exit(0 if d >= 20 else 1)" "${DB[aec]}" "${DB[off]}" || fail "echo reduction under 20 dB"

echo "crash: kill -9 the daemon while a meeting runs on the canceller"
daemon aec
$BIN meeting start --title crash >/dev/null; state recording; sleep 1
pactl list short sources | grep -q voxtype_meeting_aec_source || fail "the canceller's source is not up"
kill -9 $DPID; sleep 2
pactl list short sources | grep -q voxtype_meeting_aec_source && fail "the canceller outlived the daemon"
pgrep -f "[m]eeting-aec.conf" >/dev/null && fail "a canceller process is still running"
echo "  after kill -9: source gone, no canceller process"

kill %1 %2 %3 2>/dev/null
[ $FAIL = 0 ] && echo PASS || { echo "logs in $OUT"; exit 1; }
