# Audio Feedback on Transcription Completion (#258)

Verifies the TranscriptionComplete sound event exists and is wired in.

```bash
# Structural verification
grep -c "TranscriptionComplete" src/audio/feedback.rs src/daemon.rs
# Expected: 2+ in feedback.rs, 2+ in daemon.rs

# Runtime test (requires audio feedback enabled):
# 1. Set [audio.feedback] enabled = true, theme = "default" in config.toml
# 2. Restart daemon
# 3. Record and transcribe
# Expected: THREE distinct sounds - start beep, stop beep, completion ping
# Previously only start and stop played
```

