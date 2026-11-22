#!/usr/bin/env python3
"""Build release-sync.json from the 1.2.1 feature demo's meeting audio.

The demo played two TTS voices: "you" into the meeting microphone and
"Dana" to the speakers, with 12% of Dana mixed into the microphone 30 ms
late as echo. This script separates the two again (microphone minus the
scaled, delayed remote is the clean own voice), keeps each voice's level
per 30 ms frame, finds the utterances, and pairs them with the lines the
demo transcribed: the remote's own transcript and, for each remote
utterance, the echo fragment that turned up under "You".

The meeting tests rebuild 16 kHz audio from these levels, so the echo
guard sees the demo's real speech envelopes and echo path.

Usage: make_release_sync.py <tts-wav dir> > release-sync.json
"""

import json
import math
import struct
import sys
import wave

FRAME_S = 0.03
BLEED_GAIN = 0.12
BLEED_DELAY_S = 0.03

OWN_LINES = [
    "Okay, let's start the release sync.",
    "The build for 1.2.1 passed on every variant last night.",
    "Good. Are there any blockers left before we tag it?",
    "Agreed. Let's put it in the release notes and fix it in the next patch.",
    "Perfect, then we tag tomorrow morning at 10. Thanks, Dana.",
]
# The remote transcript as the demo's loopback track produced it, and the
# misheard echo of each line found on the microphone track (the 1.3.0 demo
# replay and exports/f1-release-sync-export.md).
REMOTE_LINES = [
    (
        "Great! I tested the Amarky package this morning, and the settings "
        "panel opened without any problems.",
        "Omarky problem.",
    ),
    (
        "Only one. The restart banner stays visible after the demon restarts. "
        "I think we can ship with that as a known issue.",
        "after the daemon restarts.",
    ),
    (
        "I'll draft the notes today and send them to you by five.",
        "5.",
    ),
]


def read(path):
    with wave.open(path) as w:
        n = w.getnframes()
        rate = w.getframerate()
        data = struct.unpack("<%dh" % n, w.readframes(n))
    return [x / 32768 for x in data], rate


def levels(samples, rate):
    n = int(rate * FRAME_S)
    out = []
    for i in range(0, len(samples), n):
        frame = samples[i : i + n]
        out.append(math.sqrt(sum(v * v for v in frame) / len(frame)))
    return out


def spans(env, floor=0.005, gap_s=0.6):
    found, cur = [], None
    for i, v in enumerate(env):
        t = i * FRAME_S
        if v > floor:
            if cur is None:
                cur = [t, t]
            cur[1] = t + FRAME_S
        elif cur and t - cur[1] > gap_s:
            found.append(cur)
            cur = None
    if cur:
        found.append(cur)
    return found


def main():
    root = sys.argv[1]
    mic, rate = read(root + "/meeting_mic.wav")
    remote, _ = read(root + "/meeting_remote.wav")
    delay = int(rate * BLEED_DELAY_S)
    own = [
        m - (BLEED_GAIN * remote[i - delay] if i >= delay else 0.0)
        for i, m in enumerate(mic)
    ]
    own_env, remote_env = levels(own, rate), levels(remote, rate)
    own_spans, remote_spans = spans(own_env), spans(remote_env)
    assert len(own_spans) == len(OWN_LINES), own_spans
    assert len(remote_spans) == len(REMOTE_LINES), remote_spans

    utterances = []
    for (a, b), text in zip(own_spans, OWN_LINES):
        utterances.append({"track": "mic", "start_ms": round(a * 1000), "end_ms": round(b * 1000), "text": text})
    for (a, b), (text, heard) in zip(remote_spans, REMOTE_LINES):
        utterances.append(
            {
                "track": "loopback",
                "start_ms": round(a * 1000),
                "end_ms": round(b * 1000),
                "text": text,
                "heard_on_mic": heard,
            }
        )
    fixture = {
        "source": "1.2.1 feature demo, tts-wav/meeting_mic.wav and meeting_remote.wav",
        "frame_ms": round(FRAME_S * 1000),
        "bleed_gain": BLEED_GAIN,
        "bleed_delay_ms": round(BLEED_DELAY_S * 1000),
        "utterances": utterances,
        "own_rms": [round(v, 4) for v in own_env],
        "remote_rms": [round(v, 4) for v in remote_env],
    }
    json.dump(fixture, sys.stdout, indent=None, separators=(",", ":"))
    sys.stdout.write("\n")


if __name__ == "__main__":
    main()
