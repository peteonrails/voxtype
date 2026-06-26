# Recording Queue

Queueing applies only to normal batch dictation. Use file output and a
temporary post-process delay so the second recording can start while the first
one is still busy.

```bash
# 1. Back up config, then edit ~/.config/voxtype/config.toml so these values
#    are present. Avoid duplicating existing TOML tables.
#
# [recording]
# queue_enabled = true
# queue_size = 2
#
# [output]
# mode = "file"
# file_path = "/tmp/voxtype-queue-smoke.txt"
# file_mode = "append"
#
# [output.post_process]
# command = "sh -c 'sleep 5; cat'"
# trim = true
# fallback_on_empty = true

systemctl --user restart voxtype
rm -f /tmp/voxtype-queue-smoke.txt

# 2. Record utterance A and stop it.
voxtype record start
sleep 3
voxtype record stop

# 3. Immediately record utterance B while A is still transcribing/post-processing.
voxtype record start
sleep 3
voxtype record stop

# 4. Verify both outputs appear, with A before B.
cat /tmp/voxtype-queue-smoke.txt
```

Negative queue cases:

```bash
# queue_size = 0 or 1 disables queueing even when queue_enabled = true.
# Restart the daemon, stop utterance A, then immediately try to start B.
# Expected: B follows the previous single-flight behavior instead of queueing.

# With queue_size = 2, start and stop A, then start and stop B while A is busy.
# Immediately try to start C before A finishes.
# Expected: C is rejected while the stopped queue is full.

# Enable eager processing or a streaming backend with queue_enabled = true.
# Expected: startup logs warn that queueing is ignored and behavior remains
# single-flight for that mode.
journalctl --user -u voxtype --since "1 minute ago" | grep -i queue
```
