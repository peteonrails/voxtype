# File Output

Tests the file output mode for writing transcriptions to files instead of typing.

## CLI File Output with Explicit Path

```bash
# Write transcription to a specific file
voxtype record start --file=/tmp/transcription.txt
sleep 3
voxtype record stop

# Verify file was created and contains text
cat /tmp/transcription.txt

# Check logs for file output:
journalctl --user -u voxtype --since "30 seconds ago" | grep -i "file"
```

## CLI File Output with Config Path

```bash
# 1. Configure file_path in config.toml:
#    [output]
#    file_path = "/tmp/voxtype-output.txt"

# 2. Restart daemon
systemctl --user restart voxtype

# 3. Use --file without a path (uses config's file_path)
voxtype record start --file
sleep 3
voxtype record stop

# 4. Verify file was created
cat /tmp/voxtype-output.txt
```

## Config-Based File Output

```bash
# 1. Configure file output mode in config.toml:
#    [output]
#    mode = "file"
#    file_path = "/tmp/voxtype-transcriptions.txt"

# 2. Restart daemon
systemctl --user restart voxtype

# 3. Record and transcribe (no CLI flags needed)
voxtype record start
sleep 3
voxtype record stop

# 4. Verify file was written
cat /tmp/voxtype-transcriptions.txt

# 5. Check logs for file output mode:
journalctl --user -u voxtype --since "30 seconds ago" | grep -E "file|output"
```

## File Append Mode

```bash
# 1. Configure append mode in config.toml:
#    [output]
#    mode = "file"
#    file_path = "/tmp/voxtype-log.txt"
#    file_mode = "append"

# 2. Clear any existing file
rm -f /tmp/voxtype-log.txt

# 3. Restart daemon
systemctl --user restart voxtype

# 4. Do multiple recordings
voxtype record start && sleep 2 && voxtype record stop
voxtype record start && sleep 2 && voxtype record stop
voxtype record start && sleep 2 && voxtype record stop

# 5. Verify all transcriptions are in file (not just the last one)
wc -l /tmp/voxtype-log.txt  # Should show multiple lines
cat /tmp/voxtype-log.txt
```

## File Overwrite Mode (Default)

```bash
# 1. Configure overwrite mode in config.toml:
#    [output]
#    mode = "file"
#    file_path = "/tmp/voxtype-overwrite.txt"
#    file_mode = "overwrite"

# 2. Restart daemon
systemctl --user restart voxtype

# 3. First recording
voxtype record start && sleep 2 && voxtype record stop
cat /tmp/voxtype-overwrite.txt
FIRST_CONTENT=$(cat /tmp/voxtype-overwrite.txt)

# 4. Second recording (should overwrite)
voxtype record start && sleep 2 && voxtype record stop
cat /tmp/voxtype-overwrite.txt

# 5. Verify file only contains the second transcription
# The content should be different (or same length, not doubled)
```

## CLI --file with Append Config

```bash
# When config has file_mode = "append", CLI --file respects it

# 1. Configure append mode:
#    [output]
#    file_mode = "append"

# 2. Restart daemon
systemctl --user restart voxtype

# 3. Use CLI with explicit path
rm -f /tmp/cli-append-test.txt
voxtype record start --file=/tmp/cli-append-test.txt
sleep 2
voxtype record stop
voxtype record start --file=/tmp/cli-append-test.txt
sleep 2
voxtype record stop

# 4. Both transcriptions should be in file
wc -l /tmp/cli-append-test.txt
```

## Directory Creation

```bash
# File output should create parent directories if needed

# 1. Remove test directory if exists
rm -rf /tmp/voxtype-test-dir

# 2. Record with a path in a non-existent directory
voxtype record start --file=/tmp/voxtype-test-dir/subdir/output.txt
sleep 2
voxtype record stop

# 3. Verify directory was created and file exists
ls -la /tmp/voxtype-test-dir/subdir/
cat /tmp/voxtype-test-dir/subdir/output.txt
```

## File Output Error Handling

```bash
# Test behavior with unwritable paths

# 1. Try to write to a read-only location
voxtype record start --file=/root/cannot-write.txt
sleep 2
voxtype record stop

# 2. Check logs for error handling:
journalctl --user -u voxtype --since "30 seconds ago" | grep -iE "error|permission"
# Expected: error message about permission denied, falls back to clipboard
```

