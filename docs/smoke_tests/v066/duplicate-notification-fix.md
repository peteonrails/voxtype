# Duplicate Notification Fix (#268)

Verifies driver-level notifications were removed (daemon handles them).

```bash
# Structural verification - no notify code in drivers
echo "ydotool.rs:" $(grep -c "send_notification\|self\.notify" src/output/ydotool.rs)
echo "dotool.rs:" $(grep -c "send_notification\|self\.notify" src/output/dotool.rs)
echo "clipboard.rs:" $(grep -c "send_notification\|self\.notify" src/output/clipboard.rs)
echo "xclip.rs:" $(grep -c "send_notification\|self\.notify" src/output/xclip.rs)
# Expected: all 0

# Runtime test (requires on_transcription = true):
# 1. Set [output.notification] on_transcription = true in config.toml
# 2. Restart daemon, record and transcribe
# 3. Verify exactly ONE notification appears (not two)
```

