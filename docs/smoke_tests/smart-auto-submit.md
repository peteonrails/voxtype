# Smart Auto-Submit

Tests the `smart_auto_submit` feature: saying "submit" at the end of dictation
strips the word and presses Enter.

## Config-based

```bash
# 1. Enable in config.toml:
#    [text]
#    smart_auto_submit = true

# 2. Restart daemon
systemctl --user restart voxtype

# 3. Record and say "hello world submit" (or "hello world submit.")
voxtype record start
sleep 4
voxtype record stop

# 4. Expected: "hello world" is typed and Enter is pressed
#
# To verify via logs, the daemon must be running with debug logging (-v):
#   journalctl --user -u voxtype --since "30 seconds ago" | grep "Smart auto-submit triggered"
# At default log level the trigger fires silently - verify by observing Enter being pressed.
```

## CLI override (per-recording)

```bash
# Force on for this recording (even if config has smart_auto_submit = false)
voxtype record start --smart-auto-submit
sleep 4
voxtype record stop
# Say "hello world submit" - should type "hello world" and press Enter

# Force off for this recording (even if config has smart_auto_submit = true)
voxtype record start --no-smart-auto-submit
sleep 4
voxtype record stop
# Say "hello world submit" - "submit" should remain in output, no Enter pressed
```

## Environment variable

```bash
# Stop the managed daemon first to avoid running two daemons simultaneously
systemctl --user stop voxtype

# Start a temporary daemon with the env var
VOXTYPE_SMART_AUTO_SUBMIT=true voxtype daemon &
DAEMON_PID=$!
sleep 2

voxtype record start && sleep 4 && voxtype record stop
# Say "hello world submit" - should type "hello world" and press Enter

# Clean up: stop the temp daemon and restart the managed one
kill $DAEMON_PID
systemctl --user start voxtype
```

## Negative cases

```bash
# "submitted" (partial word) should NOT trigger
voxtype record start --smart-auto-submit
sleep 4
voxtype record stop
# Say "I submitted the form" - full text including "submitted" should appear, no Enter

# "submit" in the middle should NOT trigger
voxtype record start --smart-auto-submit
sleep 4
voxtype record stop
# Say "please submit this form now" - full text should appear, no Enter
```

