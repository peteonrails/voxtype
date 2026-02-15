# Smoke Tests

Run these tests after installing a new build to verify core functionality.

## Basic Verification

```bash
# Version and help
voxtype --version
voxtype --help
voxtype daemon --help
voxtype record --help
voxtype setup --help

# Show current config
voxtype config

# Check status
voxtype status
```

## Recording Cycle

```bash
# Basic record start/stop
voxtype record start
sleep 3
voxtype record stop

# Toggle mode
voxtype record toggle  # starts recording
sleep 3
voxtype record toggle  # stops and transcribes

# Cancel recording (should not transcribe)
voxtype record start
sleep 2
voxtype record cancel
# Verify no transcription in logs:
journalctl --user -u voxtype --since "30 seconds ago" | grep -i transcri
```

## CLI Overrides

```bash
# Output mode override (use --clipboard, --type, or --paste)
voxtype record start --clipboard
sleep 2
voxtype record stop
# Verify clipboard has text: wl-paste

# Model override (requires model to be downloaded)
# Note: --model flag is on the main command, not record subcommand
voxtype --model base.en record start
sleep 2
voxtype record stop
```

## File Output

Tests the file output mode for writing transcriptions to files instead of typing.

### CLI File Output with Explicit Path

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

### CLI File Output with Config Path

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

### Config-Based File Output

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

### File Append Mode

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

### File Overwrite Mode (Default)

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

### CLI --file with Append Config

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

### Directory Creation

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

### File Output Error Handling

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

## GPU Isolation Mode

Tests subprocess-based GPU memory release (for laptops with hybrid graphics):

```bash
# 1. Enable gpu_isolation in config.toml:
#    [whisper]
#    gpu_isolation = true

# 2. Restart daemon
systemctl --user restart voxtype

# 3. Record and transcribe
voxtype record start && sleep 3 && voxtype record stop

# 4. Check logs for subprocess spawning:
journalctl --user -u voxtype --since "1 minute ago" | grep -i subprocess

# 5. Verify GPU memory is released after transcription:
#    (AMD) watch -n1 "cat /sys/class/drm/card*/device/mem_info_vram_used"
#    (NVIDIA) nvidia-smi
```

## On-Demand Model Loading

Tests loading model only when needed (reduces idle memory):

```bash
# 1. Enable on_demand_loading in config.toml:
#    [whisper]
#    on_demand_loading = true

# 2. Restart daemon
systemctl --user restart voxtype

# 3. Check memory before recording (model not loaded):
systemctl --user status voxtype | grep Memory

# 4. Record and transcribe
voxtype record start && sleep 3 && voxtype record stop

# 5. Check logs for model load/unload:
journalctl --user -u voxtype --since "1 minute ago" | grep -E "Loading|Unloading"
```

## Eager Processing

Tests parallel transcription of audio chunks during recording:

```bash
# 1. Enable eager processing in config.toml:
#    [whisper]
#    eager_processing = true
#    eager_chunk_secs = 3.0  # Use short chunks for visible testing
#    eager_overlap_secs = 0.5

# 2. Restart daemon
systemctl --user restart voxtype

# 3. Record for 10+ seconds (to generate multiple chunks)
voxtype record start
sleep 12
voxtype record stop

# 4. Check logs for chunk processing:
journalctl --user -u voxtype --since "1 minute ago" | grep -iE "eager|chunk"
# Expected: "Spawning eager transcription for chunk 0"
#           "Spawning eager transcription for chunk 1"
#           "Chunk 0 completed"
#           "Combined eager chunks"

# 5. Verify combined output is coherent (no obvious word duplication)
# The final transcription should read naturally

# 6. Test cancellation during eager recording
voxtype record start
sleep 5
voxtype record cancel
journalctl --user -u voxtype --since "30 seconds ago" | grep -iE "cancel|abort"
# Expected: chunk tasks are cancelled, no transcription output

# 7. Restore default (disabled) when done testing:
#    [whisper]
#    eager_processing = false
```

## Voice Activity Detection

Tests VAD filtering of silence-only recordings before transcription.

### VAD Model Setup

```bash
# Check VAD model status
voxtype setup vad --status

# Download the Silero VAD model (required for Whisper VAD backend)
voxtype setup vad
# Expected: downloads ggml-silero-vad.bin to models directory
```

### Energy VAD (No Model Required)

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

### Whisper VAD Backend

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

### Auto Backend Selection

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

### VAD Threshold Tuning

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

### VAD with Transcribe Command

```bash
# VAD can also filter files passed to the transcribe command

# Transcribe a silent WAV file (should be rejected)
voxtype transcribe --vad /path/to/silence.wav
# Expected: "No speech detected" message, no transcription output

# Transcribe a speech WAV file (should proceed normally)
voxtype transcribe --vad /path/to/speech.wav
# Expected: normal transcription output
```

### VAD Disabled (Default)

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

## Model Switching

```bash
# Download a different model if not present
voxtype setup model  # Interactive selection

# Or specify directly
voxtype setup model small.en

# Test with different models (edit config.toml or use --model flag)
```

## Remote Transcription

```bash
# 1. Configure remote backend in config.toml:
#    [whisper]
#    backend = "remote"
#    remote_endpoint = "http://your-server:8080"

# 2. Restart and test
systemctl --user restart voxtype
voxtype record start && sleep 3 && voxtype record stop

# 3. Check logs for remote transcription:
journalctl --user -u voxtype --since "1 minute ago" | grep -i remote
```

## Output Drivers

The output fallback chain is: wtype -> dotool -> ydotool -> clipboard

```bash
# Test wtype (Wayland native, default)
# Should work by default on Wayland - check logs confirm wtype is used:
voxtype record start && sleep 2 && voxtype record stop
journalctl --user -u voxtype --since "30 seconds ago" | grep -E "wtype|Text output"

# Test clipboard mode
# Edit config.toml: mode = "clipboard"
systemctl --user restart voxtype
voxtype record start && sleep 2 && voxtype record stop
wl-paste  # Should show transcribed text

# Test paste mode (clipboard + Ctrl+V)
# Edit config.toml: mode = "paste"
systemctl --user restart voxtype
voxtype record start && sleep 2 && voxtype record stop
```

## dotool Fallback

Tests the dotool output driver (supports keyboard layouts for non-US keyboards):

```bash
# Requires: dotool installed, user in 'input' group

# 1. Temporarily hide wtype to force dotool fallback
sudo mv /usr/bin/wtype /usr/bin/wtype.bak

# 2. Record and transcribe
voxtype record start && sleep 2 && voxtype record stop

# 3. Check logs for dotool usage:
journalctl --user -u voxtype --since "30 seconds ago" | grep -E "dotool|Text output"
# Expected: "wtype not available, trying next" then "Text typed via dotool"

# 4. Restore wtype
sudo mv /usr/bin/wtype.bak /usr/bin/wtype
```

## dotool Keyboard Layout

Tests keyboard layout support for non-US keyboards:

```bash
# 1. Add keyboard layout to config.toml:
#    [output]
#    dotool_xkb_layout = "de"        # German layout
#    dotool_xkb_variant = "nodeadkeys"  # Optional variant

# 2. Hide wtype to force dotool
sudo mv /usr/bin/wtype /usr/bin/wtype.bak

# 3. Restart daemon and test
systemctl --user restart voxtype
voxtype record start && sleep 2 && voxtype record stop

# 4. Verify layout is applied (check dotool receives DOTOOL_XKB_LAYOUT env var):
journalctl --user -u voxtype --since "30 seconds ago" | grep -i "keyboard layout"

# 5. Restore wtype
sudo mv /usr/bin/wtype.bak /usr/bin/wtype
```

## ydotool Fallback

Tests the ydotool output driver (requires ydotoold daemon):

```bash
# Requires: ydotool installed, ydotoold running

# 1. Temporarily hide wtype and dotool to force ydotool fallback
sudo mv /usr/bin/wtype /usr/bin/wtype.bak
sudo mv /usr/bin/dotool /usr/bin/dotool.bak

# 2. Record and transcribe
voxtype record start && sleep 2 && voxtype record stop

# 3. Check logs for ydotool usage:
journalctl --user -u voxtype --since "30 seconds ago" | grep -E "ydotool|Text output"
# Expected: "dotool not available, trying next" then "Text output via ydotool"

# 4. Restore wtype and dotool
sudo mv /usr/bin/wtype.bak /usr/bin/wtype
sudo mv /usr/bin/dotool.bak /usr/bin/dotool
```

## Output Chain Verification

Verify the complete fallback chain works:

```bash
# Check which output methods are available:
voxtype config | grep -A10 "Output Chain"

# Expected output shows installed status for each method:
#   wtype:    installed
#   dotool:   installed (if available)
#   ydotool:  installed, daemon running
#   wl-copy:  installed
```

## Delay Options

```bash
# Test type delays (edit config.toml):
#    type_delay_ms = 50       # Inter-keystroke delay
#    pre_type_delay_ms = 200  # Pre-typing delay

systemctl --user restart voxtype
voxtype record start && sleep 2 && voxtype record stop

# Check debug logs for delay application:
journalctl --user -u voxtype --since "30 seconds ago" | grep -E "delay|sleeping"
```

## Audio Feedback

```bash
# Enable audio feedback in config.toml:
#    [audio.feedback]
#    enabled = true
#    theme = "default"
#    volume = 0.5

systemctl --user restart voxtype
voxtype record start  # Should hear start beep
sleep 2
voxtype record stop   # Should hear stop beep
```

## Compositor Hooks

```bash
# Verify hooks run (check Hyprland submap changes):
voxtype record start
hyprctl submap  # Should show voxtype_recording
sleep 2
voxtype record stop
hyprctl submap  # Should show empty (reset)
```

## Transcribe Command (File Input)

```bash
# Transcribe a WAV file directly (useful for testing without mic)
voxtype transcribe /path/to/audio.wav

# With model override
voxtype transcribe --model large-v3-turbo /path/to/audio.wav
```

## Multilingual Model Verification

Tests that non-.en models load correctly and detect language:

```bash
# Use a multilingual model (without .en suffix)
voxtype --model small record start
sleep 3
voxtype record stop

# Check logs for language auto-detection:
journalctl --user -u voxtype --since "30 seconds ago" | grep "auto-detected language"

# Verify model menu shows multilingual options:
echo "0" | voxtype setup model  # Should show tiny, base, small, medium (multilingual)
```

## Invalid Model Rejection

Verify bad model names warn and fall back to default:

```bash
# Should warn, send notification, and fall back to default model
voxtype --model nonexistent record start
sleep 2
voxtype record cancel

# Expected behavior:
# 1. Warning logged: "Unknown model 'nonexistent', using default model 'base.en'"
# 2. Desktop notification via notify-send
# 3. Recording proceeds with the default model

# Check logs for warning:
journalctl --user -u voxtype --since "30 seconds ago" | grep -i "unknown model"

# The setup --set command should still reject invalid models:
voxtype setup model --set nonexistent
# Expected: error about model not installed
```

## GPU Backend Switching

Test transitions between CPU and GPU backends (engine-aware):

```bash
# Check current status
voxtype setup gpu

# Whisper mode (symlink points to voxtype-vulkan or voxtype-avx*)
# --enable switches to Vulkan, --disable switches to best CPU
ls -la /usr/bin/voxtype  # Verify current symlink
sudo voxtype setup gpu --enable   # Switch to Vulkan
ls -la /usr/bin/voxtype  # Should point to voxtype-vulkan
sudo voxtype setup gpu --disable  # Switch to best CPU (avx512 or avx2)
ls -la /usr/bin/voxtype  # Should point to voxtype-avx512 or voxtype-avx2

# Parakeet mode (symlink points to voxtype-parakeet-*)
# --enable switches to CUDA, --disable switches to best Parakeet CPU
sudo ln -sf /usr/lib/voxtype/voxtype-parakeet-avx512 /usr/bin/voxtype
sudo voxtype setup gpu --enable   # Switch to Parakeet CUDA
ls -la /usr/bin/voxtype  # Should point to voxtype-parakeet-cuda
sudo voxtype setup gpu --disable  # Switch to best Parakeet CPU
ls -la /usr/bin/voxtype  # Should point to voxtype-parakeet-avx512

# Restore to Whisper Vulkan for normal use
sudo ln -sf /usr/lib/voxtype/voxtype-vulkan /usr/bin/voxtype
```

## Multi-GPU Selection (v0.5.1)

Tests GPU selection on systems with multiple GPUs (e.g., integrated + discrete):

```bash
# Check detected GPUs
voxtype setup gpu
# Expected: lists all detected GPUs with vendor names

# Test GPU selection via environment variable
VOXTYPE_VULKAN_DEVICE=amd voxtype setup gpu | grep "GPU selection"
# Expected: "GPU selection: AMD (via VOXTYPE_VULKAN_DEVICE)"

VOXTYPE_VULKAN_DEVICE=nvidia voxtype setup gpu | grep "GPU selection"
# Expected: "GPU selection: NVIDIA (via VOXTYPE_VULKAN_DEVICE)"

VOXTYPE_VULKAN_DEVICE=intel voxtype setup gpu | grep "GPU selection"
# Expected: "GPU selection: Intel (via VOXTYPE_VULKAN_DEVICE)"

# Test with Vulkan binary
sudo ln -sf /usr/lib/voxtype/voxtype-vulkan /usr/local/bin/voxtype
systemctl --user restart voxtype

# Record with specific GPU selected
VOXTYPE_VULKAN_DEVICE=amd voxtype record start
sleep 2
voxtype record stop

# Check logs for GPU selection
journalctl --user -u voxtype --since "30 seconds ago" | grep -i "GPU selection"
```

## Whisper CLI Backend (v0.5.1)

Tests the whisper-cli subprocess backend for glibc 2.42+ compatibility:

```bash
# Requires: whisper-cli installed (from whisper.cpp project)
which whisper-cli || echo "whisper-cli not installed - skip this test"

# 1. Configure CLI backend in config.toml:
#    [whisper]
#    backend = "cli"
#    # Optionally specify path:
#    # cli_path = "/usr/local/bin/whisper-cli"

# 2. Restart daemon
systemctl --user restart voxtype

# 3. Record and transcribe
voxtype record start && sleep 3 && voxtype record stop

# 4. Check logs for CLI backend usage:
journalctl --user -u voxtype --since "30 seconds ago" | grep -i "cli"
# Expected: "Using whisper-cli subprocess backend"

# 5. Restore local backend:
#    [whisper]
#    backend = "local"
```

## Parakeet with Preloaded Model (v0.5.1)

Tests that Parakeet works correctly when `on_demand_loading = false` (the default):

```bash
# This test verifies the v0.5.1 bug fix where Parakeet would incorrectly
# use Whisper when on_demand_loading was disabled.

# 1. Verify Parakeet is configured
grep "engine" ~/.config/voxtype/config.toml
# Expected: engine = "parakeet"

# 2. Verify on_demand_loading is false (or absent, defaulting to false)
grep "on_demand_loading" ~/.config/voxtype/config.toml || echo "on_demand_loading not set (defaults to false)"

# 3. Restart daemon and check model loading
systemctl --user restart voxtype
journalctl --user -u voxtype --since "10 seconds ago" | grep -E "Loading|Parakeet"
# Expected: "Loading Parakeet Tdt model from..."
# Expected: "Parakeet Tdt model loaded in X.XXs"

# 4. Record and transcribe
voxtype record start && sleep 2 && voxtype record stop

# 5. Verify Parakeet was used (NOT Whisper)
journalctl --user -u voxtype --since "10 seconds ago" | grep -E "Transcribing.*Parakeet"
# Expected: "Transcribing X.XXs of audio (XXXXX samples) with Parakeet Tdt"

# 6. Verify NO whisper_init_state messages (indicates bug)
journalctl --user -u voxtype --since "1 minute ago" | grep -c "whisper_init_state"
# Expected: 0 (no Whisper initialization when using Parakeet)
```

## Parakeet Backend Switching

Test switching between Whisper and Parakeet engines:

```bash
# Check current status
voxtype setup parakeet

# Enable Parakeet (switches symlink to best parakeet binary)
sudo voxtype setup parakeet --enable
ls -la /usr/bin/voxtype  # Should point to voxtype-parakeet-cuda or voxtype-parakeet-avx*

# Disable Parakeet (switches back to equivalent Whisper binary)
sudo voxtype setup parakeet --disable
ls -la /usr/bin/voxtype  # Should point to voxtype-vulkan or voxtype-avx*

# Verify systemd service was updated
grep ExecStart ~/.config/systemd/user/voxtype.service
```

## Engine Switching via Model Selection

Test that selecting a model from a different engine updates config correctly:

```bash
# Start with Whisper engine configured
grep engine ~/.config/voxtype/config.toml  # Should show engine = "whisper" or be absent

# Select a Parakeet model (requires --features parakeet build)
voxtype setup model  # Choose a parakeet-tdt model
grep engine ~/.config/voxtype/config.toml  # Should show engine = "parakeet"
grep -A2 "\[parakeet\]" ~/.config/voxtype/config.toml  # Should show model name

# Select a Whisper model
voxtype setup model  # Choose a Whisper model (e.g., base.en)
grep engine ~/.config/voxtype/config.toml  # Should show engine = "whisper"

# Verify star indicator shows current model
voxtype setup model  # Current model should have * prefix
```

## Waybar JSON Output

Test the status follower with JSON format for Waybar integration:

```bash
# Should output JSON status updates (Ctrl+C to stop)
timeout 3 voxtype status --follow --format json || true

# Expected output format:
# {"text":"idle","class":"idle","tooltip":"Voxtype: idle"}

# Test during recording:
voxtype record start &
sleep 1
timeout 2 voxtype status --follow --format json || true
voxtype record cancel
```

## Single Instance Enforcement

Verify only one daemon can run at a time:

```bash
# With daemon already running via systemd, try starting another:
voxtype daemon
# Should fail with error about existing instance / PID lock

# Check PID file:
cat ~/.local/share/voxtype/voxtype.pid
ps aux | grep voxtype
```

## Post-Processing Command

Tests LLM cleanup if configured:

```bash
# 1. Configure post-processing in config.toml:
#    [output]
#    post_process_command = "your-llm-cleanup-script"

# 2. Restart daemon
systemctl --user restart voxtype

# 3. Record and transcribe
voxtype record start && sleep 3 && voxtype record stop

# 4. Check logs for post-processing:
journalctl --user -u voxtype --since "1 minute ago" | grep -i "post.process"
```

## Config Validation

Verify malformed config files produce clear errors:

```bash
# Backup current config
cp ~/.config/voxtype/config.toml ~/.config/voxtype/config.toml.bak

# Test with invalid TOML syntax
echo "invalid toml [[[" >> ~/.config/voxtype/config.toml
voxtype config  # Should show parse error with line number

# Test with unknown field (should warn but continue)
echo 'unknown_field = "value"' >> ~/.config/voxtype/config.toml
voxtype config

# Restore config
mv ~/.config/voxtype/config.toml.bak ~/.config/voxtype/config.toml
```

## Signal Handling

Test direct signal control of the daemon:

```bash
# Get daemon PID
DAEMON_PID=$(cat ~/.local/share/voxtype/voxtype.pid)

# Start recording via SIGUSR1
kill -USR1 $DAEMON_PID
voxtype status  # Should show "recording"
sleep 2

# Stop recording via SIGUSR2
kill -USR2 $DAEMON_PID
voxtype status  # Should show "transcribing" then "idle"

# Check logs:
journalctl --user -u voxtype --since "30 seconds ago" | grep -E "USR1|USR2|signal"
```

## Rapid Successive Recordings

Stress test with quick start/stop cycles:

```bash
# Run multiple quick recordings in succession
for i in {1..5}; do
    echo "Recording $i..."
    voxtype record start
    sleep 1
    voxtype record cancel
done

# Verify daemon is still healthy
voxtype status
journalctl --user -u voxtype --since "1 minute ago" | grep -iE "error|panic"
```

## Long Recording

Test recording near the max_duration_secs limit:

```bash
# Check current max duration
voxtype config | grep max_duration

# Start a long recording (default max is 60s)
# The daemon should auto-stop at the limit
voxtype record start
echo "Recording... will auto-stop at max_duration_secs"
# Wait or manually stop before limit:
sleep 10
voxtype record stop

# To test auto-cutoff, set max_duration_secs = 5 in config and record longer
```

## Service Restart Cycle

Test systemd service restarts:

```bash
# Multiple restart cycles
for i in {1..3}; do
    echo "Restart cycle $i..."
    systemctl --user restart voxtype
    sleep 2
    voxtype status
done

# Verify clean restarts in logs:
journalctl --user -u voxtype --since "1 minute ago" | grep -E "Starting|Ready|shutdown"
```

## Quick Smoke Test Script

```bash
#!/bin/bash
# quick-smoke-test.sh - Run after new build install

set -e
echo "=== Voxtype Smoke Tests ==="

echo -n "Version: "
voxtype --version

echo -n "Status: "
voxtype status

echo "Recording 3 seconds..."
voxtype record start
sleep 3
voxtype record stop
echo "Done."

echo ""
echo "Check logs:"
journalctl --user -u voxtype --since "30 seconds ago" --no-pager | tail -10
```
