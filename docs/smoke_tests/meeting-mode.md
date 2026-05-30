# Meeting Mode

Meeting mode provides continuous transcription with speaker attribution, export, and AI summarization. These tests cover the CLI commands and daemon integration.

## Meeting Lifecycle

```bash
# Start a meeting
voxtype meeting start --title "Test Meeting"
# Expected: "Meeting started: <uuid>" in output

# Check status
voxtype meeting status
# Expected: shows Active meeting with title, duration, chunk count

# Pause the meeting
voxtype meeting pause
voxtype meeting status
# Expected: shows Paused status

# Resume the meeting
voxtype meeting resume
voxtype meeting status
# Expected: shows Active status again

# Stop the meeting
voxtype meeting stop
voxtype meeting status
# Expected: shows Completed status or "No active meeting"

# Verify in logs
journalctl --user -u voxtype --since "2 minutes ago" | grep -i meeting
```

## Meeting List and Show

```bash
# List meetings (should include the one just created)
voxtype meeting list
# Expected: table with ID, title, date, duration, status

# Show details of the most recent meeting
voxtype meeting show latest
# Expected: full metadata and transcript

# Show by UUID (copy from list output)
voxtype meeting show <uuid>
```

## Meeting Export

```bash
# Export as plain text
voxtype meeting export latest --format text
# Expected: plain text transcript output

# Export as markdown
voxtype meeting export latest --format markdown
# Expected: markdown with headers and speaker labels

# Export as JSON
voxtype meeting export latest --format json
# Expected: structured JSON with metadata and segments

# Export to file
voxtype meeting export latest --format markdown --output /tmp/meeting-export.md
cat /tmp/meeting-export.md

# Export with options
voxtype meeting export latest --format text --timestamps --speakers
```

## Meeting Delete

```bash
# Delete a meeting (use UUID from list)
voxtype meeting delete <uuid>
# Expected: "Meeting deleted" confirmation

# Verify deletion
voxtype meeting list
# Expected: deleted meeting no longer appears
```

## Speaker Labels

```bash
# Start a meeting and record some audio
voxtype meeting start --title "Label Test"
sleep 10
voxtype meeting stop

# Assign speaker labels
voxtype meeting label latest SPEAKER_00 "Alice"
voxtype meeting label latest SPEAKER_01 "Bob"

# Verify labels appear in show output
voxtype meeting show latest
# Expected: speaker labels show as "Alice", "Bob" instead of SPEAKER_00/01

# Verify labels persist in export
voxtype meeting export latest --format text --speakers
```

## AI Summarization

```bash
# Requires: Ollama running locally, or a remote summarization endpoint configured

# Summarize the latest meeting
voxtype meeting summarize latest
# Expected: summary with key points, action items, and decisions

# Check logs for summarization
journalctl --user -u voxtype --since "1 minute ago" | grep -i summar
```

## Meeting Without Title

```bash
# Start without a title (should auto-generate one from the date)
voxtype meeting start
sleep 5
voxtype meeting stop

# Verify auto-generated title in list
voxtype meeting list
# Expected: title like "Meeting 2026-02-16 14:30"
```

## Rapid Start/Stop

```bash
# Stress test: quick meeting cycles
for i in {1..3}; do
    echo "Meeting cycle $i..."
    voxtype meeting start --title "Quick $i"
    sleep 2
    voxtype meeting stop
done

# Verify all meetings were saved
voxtype meeting list
# Expected: 3 new meetings in the list

# Verify daemon is healthy
voxtype status
```

## Meeting During Active Recording

```bash
# Verify meeting mode and push-to-talk don't conflict
voxtype meeting start --title "Conflict Test"
sleep 2

# Try a push-to-talk recording while meeting is active
voxtype record start
sleep 2
voxtype record stop
# Expected: either clear error or both work independently

voxtype meeting stop
```

## Meeting Config Validation

```bash
# Verify meeting config is shown
voxtype config | grep -A20 "\[meeting\]"
# Expected: meeting section with audio, storage, diarization settings

# Test with custom chunk duration (edit config.toml):
#    [meeting.audio]
#    chunk_duration_secs = 15

# Restart and verify
systemctl --user restart voxtype
voxtype meeting start --title "Custom Chunk"
sleep 20
voxtype meeting stop
journalctl --user -u voxtype --since "1 minute ago" | grep -i chunk
# Expected: chunks processed at 15-second intervals
```

## Storage Verification

```bash
# Check where meetings are stored
ls ~/.local/share/voxtype/meetings/
# Expected: directories named like "2026-02-16-test-meeting"

# Verify SQLite index
ls ~/.local/share/voxtype/meetings/index.db
# Expected: file exists

# Verify transcript files
ls ~/.local/share/voxtype/meetings/*/transcript.json
# Expected: JSON files for completed meetings

# Verify metadata files
cat ~/.local/share/voxtype/meetings/*/metadata.json | head -20
# Expected: valid JSON with meeting metadata
```

## Error Handling

```bash
# Double-start (meeting already in progress)
voxtype meeting start --title "First"
voxtype meeting start --title "Second"
# Expected: error "Meeting already in progress"
voxtype meeting stop

# Pause when no meeting active
voxtype meeting pause
# Expected: error "No active meeting to pause"

# Resume when no meeting paused
voxtype meeting resume
# Expected: error "No paused meeting to resume"

# Stop when no meeting active
voxtype meeting stop
# Expected: error "No meeting in progress"

# Show nonexistent meeting
voxtype meeting show 00000000-0000-0000-0000-000000000000
# Expected: error "Meeting not found"

# Export with invalid format
voxtype meeting export latest --format invalid
# Expected: error about unsupported format

# Export with invalid meeting ID
voxtype meeting export not-a-uuid --format text
# Expected: error about invalid meeting ID

# Label nonexistent meeting
voxtype meeting label 00000000-0000-0000-0000-000000000000 SPEAKER_00 "Alice"
# Expected: error "Meeting not found"
```

## Dual Audio Sources

```bash
# Verify loopback detection
# 1. Configure loopback in config.toml:
#    [meeting.audio]
#    loopback_device = "auto"

# 2. Start a meeting while in a video call (Zoom, Teams, etc.)
voxtype meeting start --title "Video Call Test"

# 3. Speak into mic and wait for remote participants to speak
sleep 30
voxtype meeting stop

# 4. Check speaker attribution
voxtype meeting show latest
# Expected: segments attributed to "You" (mic) and "Remote" (loopback)

# 5. Verify export includes speaker labels
voxtype meeting export latest --format text --speakers
# Expected: "You:" and "Remote:" labels in output

# Disable loopback (mic-only mode)
#    [meeting.audio]
#    loopback_device = "disabled"
systemctl --user restart voxtype
voxtype meeting start --title "Mic Only Test"
sleep 10
voxtype meeting stop
voxtype meeting show latest
# Expected: all segments attributed to "You" or "Unknown"
```

## Diarization Backend Selection

```bash
# Simple diarization (default, source-based)
voxtype config | grep -A5 "diarization"
# Expected: backend = "simple"

# ML diarization (requires ml-diarization feature)
# 1. Configure in config.toml:
#    [meeting.diarization]
#    backend = "ml"
#    max_speakers = 4
# 2. Restart and verify
systemctl --user restart voxtype
journalctl --user -u voxtype --since "10 seconds ago" | grep -i diariz
# Expected: "Using ML diarization" or "falling back to simple" if model missing
```
