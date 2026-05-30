# Multi-Engine Transcription

Tests each available transcription engine with a WAV file. Use `tests/fixtures/vad/speech_long.wav` (English) or `tests/fixtures/sensevoice/zh.wav` (Chinese) as test audio. Each engine must be compiled in (check `voxtype --version` or build features).

## Engine Quick Test

```bash
# Test audio paths
EN_AUDIO="tests/fixtures/vad/speech_long.wav"
ZH_AUDIO="tests/fixtures/sensevoice/zh.wav"

# Whisper (always available)
voxtype transcribe --engine whisper "$EN_AUDIO"

# Parakeet (requires --features parakeet)
voxtype transcribe --engine parakeet "$EN_AUDIO"

# Moonshine (requires --features moonshine)
voxtype transcribe --engine moonshine "$EN_AUDIO"

# SenseVoice (requires --features sensevoice)
voxtype transcribe --engine sensevoice "$EN_AUDIO"
voxtype transcribe --engine sensevoice "$ZH_AUDIO"

# Paraformer (requires --features paraformer, English and Chinese models)
voxtype transcribe --engine paraformer "$EN_AUDIO"

# Dolphin (requires --features dolphin, Eastern languages only, no English)
voxtype transcribe --engine dolphin "$ZH_AUDIO"

# Omnilingual (requires --features omnilingual, 1600+ languages)
voxtype transcribe --engine omnilingual "$EN_AUDIO"
```

## Engine Daemon Integration

Test each engine running as the daemon's active engine:

```bash
# For each engine, update config.toml engine = "<name>" and restart:

# SenseVoice
# 1. Set engine = "sensevoice" in config.toml
# 2. Restart daemon
systemctl --user restart voxtype
# 3. Verify model loads
journalctl --user -u voxtype --since "10 seconds ago" | grep -iE "sensevoice|loading"
# 4. Record and transcribe
voxtype record start && sleep 3 && voxtype record stop
# 5. Check logs for correct engine
journalctl --user -u voxtype --since "30 seconds ago" | grep -i "transcri"

# Repeat for: paraformer, dolphin, omnilingual, moonshine, parakeet
# Then restore engine = "whisper" when done
```

## Engine Error Handling

```bash
# Request an engine that isn't compiled in (should give clear error)
# e.g., if built without --features dolphin:
voxtype transcribe --engine dolphin tests/fixtures/vad/speech_long.wav
# Expected: error about Dolphin not being compiled in

# Request unknown engine
voxtype transcribe --engine nonexistent tests/fixtures/vad/speech_long.wav
# Expected: error listing valid engine names

# Engine with missing model
# (temporarily rename model dir to simulate missing model)
mv ~/.local/share/voxtype/models/sensevoice-small{,.bak}
voxtype transcribe --engine sensevoice tests/fixtures/vad/speech_long.wav
# Expected: clear error with "Run: voxtype setup model"
mv ~/.local/share/voxtype/models/sensevoice-small{.bak,}
```

## Engine Performance Comparison

```bash
# Compare transcription speed across engines for the same audio file
AUDIO="tests/fixtures/vad/speech_long.wav"

for engine in whisper parakeet moonshine sensevoice paraformer omnilingual; do
    echo -n "$engine: "
    /usr/bin/time -f "%e seconds" voxtype transcribe --engine $engine "$AUDIO" 2>&1 | tail -1
done
```

