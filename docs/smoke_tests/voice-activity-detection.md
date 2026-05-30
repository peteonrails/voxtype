# Voice Activity Detection

Tests VAD filtering of silence-only recordings before transcription.

## VAD Model Setup

```bash
# Check VAD model status
voxtype setup vad --status

# Download the Silero VAD model (required for Whisper VAD backend)
voxtype setup vad
# Expected: downloads ggml-silero-vad.bin to models directory
```

## Energy VAD (No Model Required)

```bash
# 1. Enable Energy VAD in config.toml:
#    [vad]
#    enabled = true
#    backend = "energy"
#    threshold = 0.5

# 2. Restart daemon
systemctl --user restart voxtype

# 3. Check logs confirm VAD is active:
journalctl --user -u voxtype --since "10 seconds ago" | grep -i "vad"
# Expected: "Voice Activity Detection enabled (backend: Energy, threshold: 0.50, ...)"

# 4. Record silence (don't speak, cover mic)
voxtype record start
sleep 3
voxtype record stop

# 5. Verify silence was rejected:
journalctl --user -u voxtype --since "30 seconds ago" | grep -iE "vad|no speech|silence"
# Expected: "VAD: no speech detected" and cancel feedback sound
# Expected: no transcription attempt

# 6. Record with speech (speak normally)
voxtype record start
sleep 3
voxtype record stop

# 7. Verify speech was accepted:
journalctl --user -u voxtype --since "30 seconds ago" | grep -iE "vad|speech detected"
# Expected: "VAD: speech detected" followed by transcription
```

## Whisper VAD Backend

```bash
# Requires: voxtype setup vad (Silero model downloaded)

# 1. Enable Whisper VAD in config.toml:
#    [vad]
#    enabled = true
#    backend = "whisper"
#    threshold = 0.5

# 2. Restart daemon
systemctl --user restart voxtype

# 3. Check logs confirm Whisper VAD is active:
journalctl --user -u voxtype --since "10 seconds ago" | grep -i "vad"
# Expected: "Using Whisper VAD backend with model ..."

# 4. Record silence - should be rejected (same as Energy VAD test above)
voxtype record start && sleep 3 && voxtype record stop
journalctl --user -u voxtype --since "30 seconds ago" | grep -iE "vad|no speech"

# 5. Record speech - should be accepted and transcribed
voxtype record start && sleep 3 && voxtype record stop
journalctl --user -u voxtype --since "30 seconds ago" | grep -iE "vad|speech detected"
```

## Auto Backend Selection

```bash
# 1. Set backend to auto in config.toml:
#    [vad]
#    enabled = true
#    backend = "auto"

# 2. Restart daemon (with Whisper engine configured)
systemctl --user restart voxtype

# 3. Check which backend was selected:
journalctl --user -u voxtype --since "10 seconds ago" | grep -i "vad"
# Expected with Whisper engine: "Using Whisper VAD backend"
# Expected with Parakeet engine: "Using Energy VAD backend"
```

## VAD Threshold Tuning

```bash
# Test that lower thresholds accept more audio (more sensitive)

# 1. Set a very low threshold:
#    [vad]
#    enabled = true
#    backend = "energy"
#    threshold = 0.1

# 2. Restart and record quiet speech or background noise
systemctl --user restart voxtype
voxtype record start && sleep 3 && voxtype record stop
# Expected: likely accepts the recording (low threshold = sensitive)

# 3. Set a high threshold:
#    threshold = 0.9

# 4. Restart and record quiet speech
systemctl --user restart voxtype
voxtype record start && sleep 3 && voxtype record stop
# Expected: likely rejects quiet speech (high threshold = strict)

# 5. Restore default:
#    threshold = 0.5
```

## VAD with Transcribe Command

VAD configuration applies to recorded audio (record start/stop). The
`transcribe` subcommand does not expose a per-invocation `--vad` flag —
it reads the engine override only. To filter `voxtype transcribe` output
through VAD, enable VAD in `config.toml` and re-run the command.

```bash
# Verify there is no --vad flag on transcribe (regression guard)
voxtype transcribe --help 2>&1 | grep -- --vad
# Expected: no match (transcribe takes <FILE> and --engine only)
```

## VAD Disabled (Default)

```bash
# Verify VAD doesn't interfere when disabled (default behavior)

# 1. Ensure VAD is disabled (default):
#    [vad]
#    enabled = false
#    (or simply omit the [vad] section)

# 2. Restart daemon
systemctl --user restart voxtype

# 3. Record silence - should still attempt transcription (no VAD filtering)
voxtype record start && sleep 3 && voxtype record stop
journalctl --user -u voxtype --since "30 seconds ago" | grep -i "vad"
# Expected: no VAD messages in logs

# 4. Restore VAD config when done testing
```

