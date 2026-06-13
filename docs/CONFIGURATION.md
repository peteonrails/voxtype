# Voxtype Configuration Reference

Complete reference for all configuration options in Voxtype.

> **Tip**: For interactive editing, run `voxtype configure` — it edits the
> same `config.toml` this document describes, preserves comments and unknown
> fields, and validates each save before swapping the file in. The reference
> below stays useful for scripted setups, advanced fields the TUI doesn't
> surface yet, and understanding each section end-to-end. See the
> [TUI section in the user manual](USER_MANUAL.md#voxtype-configure) for
> keybindings.

## Configuration File Location

Voxtype looks for configuration in the following locations (in order):

1. Path specified via `-c` / `--config` flag
2. `~/.config/voxtype/config.toml` (XDG config directory)
3. `/etc/voxtype/config.toml` (system-wide default)
4. Built-in defaults

## Configuration Sections

---

## engine

**Type:** String
**Default:** `"whisper"`
**Required:** No

Selects which speech-to-text engine to use for transcription.

**Values:**
- `whisper` - OpenAI Whisper via whisper.cpp (default, recommended)
- `parakeet` - NVIDIA Parakeet via ONNX Runtime (requires ONNX binary)
- `moonshine` - Moonshine encoder-decoder transformer via ONNX Runtime (experimental, requires special binary)
- `sensevoice` - Alibaba SenseVoice CTC via ONNX Runtime (CJK + English)
- `paraformer` - FunASR Paraformer CTC via ONNX Runtime (Chinese + English)
- `dolphin` - Dictation-optimized CTC via ONNX Runtime (Chinese + English)
- `omnilingual` - FunASR Omnilingual CTC via ONNX Runtime (50+ languages)
- `cohere` - Cohere Transcribe encoder-decoder via ONNX Runtime (#1 Open ASR Leaderboard, 14 languages, ~3 GB model)

**Example:**
```toml
engine = "whisper"
```

**CLI override:**
```bash
voxtype --engine parakeet daemon
```

**Persistent change via CLI:**

To change the engine in your config file (preserving comments and other
settings), use:

```bash
voxtype config set engine whisper
voxtype config set engine parakeet
```

This is non-interactive equivalent of the `voxtype configure` TUI's engine
picker. It validates that the requested engine is compiled into your binary
(rebuild with `cargo build --features <engine>` or install a matching
prebuilt variant if it isn't), updates `~/.config/voxtype/config.toml`
atomically, and prints the restart hint. The daemon does not hot-reload
config changes; restart it with `systemctl --user restart voxtype` for the
new engine to take effect.

**Notes:**
- All engines except Whisper require an ONNX-enabled binary (`voxtype-*-onnx-*`)
- Each ONNX engine reads its own `[<engine>]` section (e.g. `[parakeet]`, `[cohere]`)
- See [PARAKEET.md](PARAKEET.md) for detailed Parakeet setup instructions
- See [MOONSHINE.md](MOONSHINE.md) for detailed Moonshine setup instructions
- Cohere Transcribe is the largest model voxtype ships (~3 GB int8); use `voxtype setup model` to download it

---

## [hotkey]

Controls which key triggers push-to-talk recording.

### key

**Type:** String
**Default:** `"SCROLLLOCK"`
**Required:** No

The main key to hold for recording. Must be a valid Linux evdev key name.

**Common values:**
- `SCROLLLOCK` - Scroll Lock key (recommended)
- `PAUSE` - Pause/Break key
- `RIGHTALT` - Right Alt key
- `F13` through `F24` - Extended function keys
- `MEDIA` - Media key (often a dedicated button on multimedia keyboards)
- `RECORD` - Record key
- `INSERT` - Insert key
- `HOME` - Home key
- `END` - End key
- `PAGEUP` - Page Up key
- `PAGEDOWN` - Page Down key
- `DELETE` - Delete key

**Example:**
```toml
[hotkey]
key = "PAUSE"
```

**Numeric keycodes:**

You can also specify keys by their numeric keycode if the key name isn't in the built-in list. Use a prefix to indicate the source tool, since different tools report different numbers for the same key:

- `WEV_234` or `X11_234` or `XEV_234` - XKB keycode as shown by `wev` or `xev` (offset by 8 from the kernel value)
- `EVTEST_226` - kernel keycode as shown by `evtest`
- Hex values are also accepted: `WEV_0xEA`, `EVTEST_0xE2`

Bare numeric values (e.g. `226`) are not accepted because `wev`/`xev` and `evtest` report different numbers for the same key.

**Finding key names:**
```bash
# Using evtest (shows kernel keycodes):
sudo evtest
# Select keyboard, press desired key, note KEY_XXXX name

# Using wev on Wayland (shows XKB keycodes):
wev
# Press the key, note the keycode number — use with WEV_ prefix
```

### modifiers

**Type:** Array of strings
**Default:** `[]`
**Required:** No

Additional modifier keys that must be held along with the main key.

**Valid modifiers:**
- `LEFTCTRL`, `RIGHTCTRL`
- `LEFTALT`, `RIGHTALT`
- `LEFTSHIFT`, `RIGHTSHIFT`
- `LEFTMETA`, `RIGHTMETA`

**Example:**
```toml
[hotkey]
key = "SCROLLLOCK"
modifiers = ["LEFTCTRL"]  # Requires Ctrl+ScrollLock
```

### mode

**Type:** String
**Default:** `"push_to_talk"`
**Required:** No

Activation mode for the hotkey.

**Values:**
- `push_to_talk` - Hold hotkey to record, release to transcribe (default)
- `toggle` - Press hotkey once to start recording, press again to stop

**Example:**
```toml
[hotkey]
key = "SCROLLLOCK"
mode = "toggle"  # Press to start, press again to stop
```

### enabled

**Type:** Boolean
**Default:** `true`
**Required:** No

Enable or disable the built-in hotkey detection.

When set to `false`, voxtype will not listen for keyboard events via evdev. Instead, use the `voxtype record` command to control recording from external sources like compositor keybindings.

**When to disable:**
- You prefer using your compositor's native keybindings (Hyprland, Sway)
- You don't want to add your user to the `input` group
- You want to use key combinations not supported by evdev (e.g., Super+V)

**Example:**
```toml
[hotkey]
enabled = false  # Use compositor keybindings instead
```

**Usage with compositor keybindings:**

When `enabled = false`, control recording via CLI:
```bash
voxtype record start                # Start recording
voxtype record start --file=out.txt # Write transcription to a file
voxtype record start --file         # Write to file_path from config
voxtype record stop                 # Stop and transcribe
voxtype record toggle               # Toggle recording state
```

Bind these commands in your compositor config:

**Hyprland:**
```hyprlang
bind = SUPER, V, exec, voxtype record start
bindr = SUPER, V, exec, voxtype record stop
```

**Sway:**
```
bindsym $mod+v exec voxtype record start
bindsym --release $mod+v exec voxtype record stop
```

**KDE Plasma (KWin):**

KDE does not support key-release events, so use toggle mode. Open **System Settings > Shortcuts > Custom Shortcuts**, create a new global shortcut, and set the command to `voxtype record toggle`. Assign your preferred key combination (e.g., Meta+V).

**Note:** For `toggle` mode to work correctly, you must also set `state_file = "auto"` so voxtype can track its current state.

See [User Manual - Compositor Keybindings](USER_MANUAL.md#compositor-keybindings) for complete setup instructions.

### model_modifier

**Type:** String
**Default:** None (disabled)
**Required:** No

Optional modifier key that triggers the secondary model when held while pressing the hotkey. Requires `secondary_model` to be set in the `[whisper]` section.

**Example:**
```toml
[hotkey]
key = "SCROLLLOCK"
model_modifier = "LEFTSHIFT"  # Hold Shift + hotkey for secondary model

[whisper]
model = "base.en"
secondary_model = "large-v3-turbo"
```

**Valid key names:** Same modifier keys as `modifiers` option:
- `LEFTSHIFT`, `RIGHTSHIFT`
- `LEFTCTRL`, `RIGHTCTRL`
- `LEFTALT`, `RIGHTALT`
- `LEFTMETA`, `RIGHTMETA`

**Note:** This only applies when using evdev hotkey detection (`enabled = true`). When using compositor keybindings, use `voxtype record start --model <model>` instead.

### cancel_key

**Type:** String
**Default:** None (disabled)
**Required:** No

Optional key to cancel recording or transcription in progress. When pressed, any active recording is discarded and any in-progress transcription is aborted. No text is output.

**Example:**
```toml
[hotkey]
key = "SCROLLLOCK"
cancel_key = "ESC"  # Press Escape to cancel
```

**Valid key names:** Same as the `key` option - any valid Linux evdev key name.

**Common cancel keys:**
- `ESC` - Escape key
- `BACKSPACE` - Backspace key
- `F12` - Function key

**Note:** This only applies when using evdev hotkey detection (`enabled = true`). When using compositor keybindings, use `voxtype record cancel` instead. See [User Manual - Canceling Transcription](USER_MANUAL.md#canceling-transcription).

### [hotkey.profile_modifiers]

**Type:** Table (key = modifier name, value = profile name)
**Default:** Empty (disabled)
**Required:** No

Maps modifier keys to named profiles. When a profile modifier is held while pressing the hotkey, that profile's post-processing command is used instead of the default. Profiles are defined in `[profiles.<name>]` sections.

**Example:**
```toml
[hotkey]
key = "SCROLLLOCK"

[hotkey.profile_modifiers]
RIGHTSHIFT = "translate"   # Shift + hotkey activates [profiles.translate]
RIGHTALT = "formal"        # RightAlt + hotkey activates [profiles.formal]

[profiles.translate]
post_process_command = "my-script.sh --translate-en"
post_process_timeout_ms = 10000

[profiles.formal]
post_process_command = "my-script.sh --formal"
```

**Valid key names:** Same modifier keys as `modifiers` option:
- `LEFTSHIFT`, `RIGHTSHIFT`
- `LEFTCTRL`, `RIGHTCTRL`
- `LEFTALT`, `RIGHTALT`
- `LEFTMETA`, `RIGHTMETA`

**Note:** This only applies when using evdev hotkey detection (`enabled = true`). When using compositor keybindings, use `voxtype record start --profile <name>` instead. Avoid using the same key in both `modifiers` and `profile_modifiers` -- every hotkey press would always activate that profile.

---

## [audio]

Controls audio capture settings.

### device

**Type:** String
**Default:** `"default"`
**Required:** No

The audio input device to use. Use `"default"` for the system default microphone.

**Finding device names:**
```bash
pactl list sources short
```

**Example:**
```toml
[audio]
device = "alsa_input.usb-Blue_Microphones_Yeti-00.analog-stereo"
```

### sample_rate

**Type:** Integer
**Default:** `16000`
**Required:** No

Audio sample rate in Hz. Whisper expects 16000 Hz; other rates will be resampled.

**Recommended:** Keep at `16000` unless your hardware requires otherwise.

```toml
[audio]
sample_rate = 16000
```

### max_duration_secs

**Type:** Integer
**Default:** `60`
**Required:** No

Maximum recording duration in seconds. Recording automatically stops after this limit as a safety measure. When the limit is reached, the captured audio is transcribed and output normally rather than being discarded.

**Example:**
```toml
[audio]
max_duration_secs = 120  # Allow 2-minute recordings
```

---

## [audio.feedback]

Controls audio feedback sounds (beeps when recording starts/stops).

### enabled

**Type:** Boolean
**Default:** `false`
**Required:** No

When `true`, plays audio cues when recording starts and stops.

### theme

**Type:** String
**Default:** `"default"`
**Required:** No

Sound theme to use for audio feedback.

**Built-in themes:**
- `default` - Clear, pleasant two-tone beeps
- `subtle` - Quiet, unobtrusive clicks
- `mechanical` - Typewriter/keyboard-like sounds

**Custom themes:** Specify a path to a directory containing `start.wav`, `stop.wav`, and `error.wav` files.

### volume

**Type:** Float
**Default:** `0.7`
**Required:** No

Volume level for audio feedback, from `0.0` (silent) to `1.0` (full volume).

**Example:**
```toml
[audio.feedback]
enabled = true
theme = "subtle"
volume = 0.5
```

**Custom theme example:**
```toml
[audio.feedback]
enabled = true
theme = "/home/user/.config/voxtype/sounds"
volume = 0.8
```

---

## [whisper]

Controls the Whisper speech-to-text engine.

### backend

**Type:** String
**Default:** `"local"`
**Required:** No

Selects the transcription backend.

**Values:**
- `local` - Use whisper.cpp locally via FFI bindings (default, fully offline)
- `remote` - Send audio to a remote server for transcription
- `cli` - Use whisper-cli subprocess (fallback for systems where FFI crashes)

> **Privacy Notice**: When using `remote` backend, audio is transmitted over the network. See [User Manual - Remote Whisper Servers](USER_MANUAL.md#remote-whisper-servers) for privacy considerations.

**When to use `cli` backend (Linux only):**
The `cli` backend is a workaround for systems where the whisper-rs FFI bindings crash due to C++ exceptions crossing the FFI boundary. This affects some systems with glibc 2.42+ (e.g., Ubuntu 25.10). If voxtype crashes during transcription, try the `cli` backend.

Requires `whisper-cli` from [whisper.cpp](https://github.com/ggerganov/whisper.cpp).

**Examples:**
```toml
[whisper]
backend = "remote"
remote_endpoint = "http://192.168.1.100:8080"
```

```toml
[whisper]
backend = "cli"
whisper_cli_path = "/usr/local/bin/whisper-cli"  # Optional
```

### model

**Type:** String
**Default:** `"base.en"`
**Required:** No

Which Whisper model to use for transcription.

**Model names:**
| Value | Size | Speed | Accuracy | Notes |
|-------|------|-------|----------|-------|
| `tiny` | 39 MB | Fastest | Good | Multilingual |
| `tiny.en` | 39 MB | Fastest | Better | English only |
| `base` | 142 MB | Fast | Better | Multilingual |
| `base.en` | 142 MB | Fast | Good | English only (default) |
| `small` | 466 MB | Medium | Great | Multilingual |
| `small.en` | 466 MB | Medium | Great | English only |
| `medium` | 1.5 GB | Slow | Excellent | Multilingual |
| `medium.en` | 1.5 GB | Slow | Excellent | English only |
| `large-v3` | 3.1 GB | Slowest | Best | Multilingual |
| `large-v3-turbo` | 1.6 GB | Fast | Excellent | Multilingual, GPU recommended |

**Custom model path:**
```toml
[whisper]
model = "/home/user/models/custom-whisper.bin"
```

### language

**Type:** String or Array of Strings
**Default:** `"en"`
**Required:** No

Language code for transcription. Supports three modes:

1. **Single language** - Use a specific language for all transcriptions
2. **Auto-detect** - Let Whisper detect from all ~99 supported languages
3. **Constrained auto-detect** - Detect from a specific set of allowed languages

**Common values:**
- `"en"` - English
- `"auto"` - Auto-detect from all languages
- `"es"` - Spanish
- `"fr"` - French
- `"de"` - German
- `"ja"` - Japanese
- `"zh"` - Chinese
- `["en", "fr"]` - Auto-detect between English and French only

**Examples:**
```toml
[whisper]
# Single language (fastest, most accurate for monolingual use)
language = "en"

# Auto-detect from all languages
language = "auto"

# Constrained auto-detect (recommended for multilingual users)
# Whisper sometimes misdetects language for short sentences.
# This limits detection to your known languages for better accuracy.
language = ["en", "fr"]

# Works with any number of languages
language = ["en", "fr", "de", "es"]
```

**When to use constrained auto-detect:**
- You regularly speak in 2-3 languages
- Whisper misdetects language for short sentences
- You want faster detection than full auto-detect

**Note:** Remote backends (OpenAI API) don't support language arrays. When using remote backend with an array, the first language is used.

### translate

**Type:** Boolean
**Default:** `false`
**Required:** No

When `true`, translates non-English speech to English.

**Example:**
```toml
[whisper]
language = "auto"
translate = true  # Translate everything to English
```

### threads

**Type:** Integer
**Default:** Auto-detected
**Required:** No

Number of CPU threads for Whisper inference. If omitted, automatically detects optimal thread count.

**Example:**
```toml
[whisper]
threads = 4  # Limit to 4 threads
```

**Tip:** For best performance, set to your physical core count (not hyperthreads).

### on_demand_loading

**Type:** Boolean
**Default:** `false`
**Required:** No

Controls when the Whisper model is loaded into memory.

**Values:**
- `false` (default) - Model is loaded at daemon startup and kept in memory. Provides fastest response times but uses memory/VRAM continuously.
- `true` - Model is loaded when recording starts and unloaded after transcription completes. Saves memory/VRAM but adds a brief delay when starting each recording.

**When to use `on_demand_loading = true`:**
- Running on a memory-constrained system
- Using GPU acceleration and want to free VRAM for other applications
- Running multiple GPU-accelerated applications simultaneously
- Using large models (medium, large-v3) that consume significant memory

**When to keep default (`false`):**
- Want the fastest possible response time
- Have plenty of available memory/VRAM
- Using voxtype frequently throughout the day

**Example:**
```toml
[whisper]
model = "large-v3"
on_demand_loading = true  # Free VRAM when not transcribing
```

**Performance note:** On modern systems with SSDs, model loading typically takes under 1 second for base/small models. Larger models (medium, large-v3) may take 2-3 seconds to load.

### gpu_isolation

**Type:** Boolean
**Default:** `false`
**Required:** No

GPU memory isolation mode. When enabled, transcription runs in a subprocess that exits after each recording, fully releasing GPU memory between transcriptions.

**Values:**
- `false` (default) - Model stays loaded in the daemon process. Fastest response, but GPU memory is held continuously.
- `true` - Model loads in a subprocess when recording starts, subprocess exits after transcription. Releases all GPU/VRAM between recordings.

**When to use `gpu_isolation = true`:**
- Laptops with hybrid graphics (NVIDIA Optimus, AMD switchable)
- You want the discrete GPU to power down when not transcribing
- Battery life is a priority
- You're running other GPU-intensive applications alongside Voxtype

**When to keep default (`false`):**
- Desktop systems with dedicated GPUs
- You transcribe frequently and want zero latency
- Power consumption is not a concern

**Performance impact:**

Benchmarks on AMD Radeon RX 7800 XT with large-v3-turbo:

| Mode | Transcription Latency | Idle RAM | Idle GPU Memory |
|------|----------------------|----------|-----------------|
| Standard (`false`) | 0.49s avg | ~1.6 GB | 409 MB |
| GPU Isolation (`true`) | 0.50s avg | 0 | 0 |

The model loads while you speak (0.38-0.42s), so the additional latency is only ~10ms (2%) after recording stops. The delay should be barely perceptible because model loading overlaps with speaking time.

**Example:**
```toml
[whisper]
model = "large-v3-turbo"
gpu_isolation = true  # Release GPU memory between transcriptions
```

**Note:** This setting only applies when using the local whisper backend (`backend = "local"`). It has no effect with remote transcription since no local GPU is used.

### gpu_device

**Type:** Integer
**Default:** Not set (uses device index 0)
**Required:** No

GPU device index for Vulkan/CUDA/Metal backend selection. On multi-GPU systems, whisper.cpp may select the wrong GPU by default (e.g., an integrated GPU instead of a discrete GPU), causing slower transcription.

This sets the device index passed directly to whisper.cpp. For systems where the integrated and discrete GPUs are from **different vendors** (e.g., Intel iGPU + NVIDIA dGPU), the `VOXTYPE_VULKAN_DEVICE` environment variable is usually simpler -- it filters by vendor at the Vulkan driver level. See the [Environment Variables](#environment-variables) section.

Use `gpu_device` when you need precise index-level control, such as when both GPUs are from the same vendor.

**Example:**
```toml
[whisper]
gpu_device = 1  # Use discrete GPU (skip integrated GPU at index 0)
```

**How to find your GPU index:** Run `voxtype setup gpu` to see detected GPUs, or `vulkaninfo --summary` for Vulkan device indices.

### context_window_optimization

**Type:** Boolean
**Default:** `false`
**Required:** No

Optimizes Whisper's context window size for short recordings. When enabled, clips under 22.5 seconds use a smaller context window proportional to their length, speeding up transcription. Also sets `no_context=true` to prevent phrase repetition.

**Values:**
- `false` (default) - Use Whisper's full 30-second context window (1500 tokens). Most compatible.
- `true` - Use optimized context window for short clips. Faster but may cause issues with some models.

**Performance impact:**

| Mode | ~1.5s clip (CPU) | ~1.5s clip (GPU) |
|------|------------------|------------------|
| Enabled (`true`) | ~8s | ~0.28s |
| Disabled (`false`) | ~15s | ~0.46s |

The optimization provides roughly 1.6-1.9x speedup for short recordings on both CPU and GPU.

**When to enable (`true`):**
- You want faster transcription for short clips
- Your model doesn't exhibit repetition issues (test before enabling)
- You're using smaller models (tiny, base, small) which are more stable

**When to keep disabled (`false`):**
- You use large-v3 or large-v3-turbo models (known repetition issues)
- You experience phrase repetition like "word word word"
- You want maximum compatibility across all models

**Example:**
```toml
[whisper]
model = "base.en"
context_window_optimization = true  # Enable for faster transcription
```

**CLI override:**
```bash
voxtype --whisper-context-optimization daemon
```

**Note:** This setting only applies when using the local whisper backend (`backend = "local"`). It has no effect with remote transcription.

### eager_processing

**Type:** Boolean
**Default:** `false`
**Required:** No

Enable eager input processing. When enabled, audio is split into chunks and transcribed in parallel with continued recording, reducing perceived latency on slower machines.

**Values:**
- `false` (default) - Traditional mode: record all audio, then transcribe
- `true` - Eager mode: transcribe chunks while recording continues

**How it works:**

1. While recording, audio is split into fixed-size chunks (default 5 seconds)
2. Each chunk is sent for transcription as soon as it's ready
3. Recording continues while earlier chunks are being transcribed
4. When recording stops, all chunk results are combined

**When to use eager processing:**
- You have a slower CPU where transcription takes several seconds
- You regularly dictate longer passages (10+ seconds)
- You want to minimize the delay between speaking and text output

**When to keep default (`false`):**
- You have a fast CPU or GPU acceleration
- Your recordings are typically short (under 5 seconds)
- You want maximum transcription accuracy (single-pass is more consistent)

**Example:**
```toml
[whisper]
model = "base.en"
eager_processing = true
eager_chunk_secs = 5.0    # 5 second chunks
eager_overlap_secs = 0.5  # 0.5 second overlap
```

**CLI override:**
```bash
voxtype --eager-processing daemon
```

**Note:** Eager processing is experimental. There may be occasional word duplications or omissions at chunk boundaries.

### eager_chunk_secs

**Type:** Float
**Default:** `5.0`
**Required:** No

Duration of each audio chunk in seconds when eager processing is enabled.

**Example:**
```toml
[whisper]
eager_processing = true
eager_chunk_secs = 3.0  # Shorter chunks for faster feedback
```

**CLI override:**
```bash
voxtype --eager-processing --eager-chunk-secs 3.0 daemon
```

**Trade-offs:**
- Shorter chunks: Faster feedback, but more boundary artifacts
- Longer chunks: Better accuracy, but less parallelism benefit

### eager_overlap_secs

**Type:** Float
**Default:** `0.5`
**Required:** No

Overlap duration in seconds between adjacent chunks when eager processing is enabled. Overlap helps catch words that span chunk boundaries.

**Example:**
```toml
[whisper]
eager_processing = true
eager_chunk_secs = 5.0
eager_overlap_secs = 1.0  # More overlap for better boundary handling
```

**CLI override:**
```bash
voxtype --eager-processing --eager-overlap-secs 1.0 daemon
```

**Trade-offs:**
- More overlap: Better word boundary handling, slightly more processing
- Less overlap: Faster processing, but may miss words at boundaries

### initial_prompt

**Type:** String
**Default:** None (empty)
**Required:** No

Provides context to Whisper to improve transcription accuracy for domain-specific vocabulary. The prompt hints at terminology, proper nouns, or formatting conventions that Whisper should expect in the audio.

**Why use it:**

Whisper sometimes mistranscribes uncommon words, especially:
- Technical jargon (Kubernetes, TypeScript, PostgreSQL)
- Company or product names (Voxtype, Hyprland, Waybar)
- People's names (especially non-English names)
- Acronyms and abbreviations (API, CLI, LLM)
- Domain-specific terms (medical, legal, scientific)

By providing an initial prompt with these terms, Whisper is more likely to recognize and transcribe them correctly.

**Example:**
```toml
[whisper]
model = "base.en"
initial_prompt = "Technical discussion about Rust, TypeScript, and Kubernetes."
```

**More examples:**

```toml
# Software development context
initial_prompt = "Voxtype, Hyprland, Waybar, Sway, wtype, ydotool, systemd, journalctl."

# Medical dictation
initial_prompt = "Medical notes. Terms: hypertension, myocardial infarction, CT scan, MRI."

# Meeting with specific attendees
initial_prompt = "Meeting with Zhang Wei, François Dupont, and Priya Sharma."
```

**CLI override:**
```bash
voxtype --initial-prompt "Discussion about Kubernetes and Terraform" daemon
```

**Tips:**
- Keep prompts concise (a few words or a short sentence)
- List specific terms you expect to appear in your dictation
- Update the prompt when your context changes (different project, different domain)
- The prompt doesn't need to be grammatically correct—a list of terms works well

**Note:** This setting only applies when using the local whisper backend (`backend = "local"`). Remote servers may ignore the initial_prompt parameter.

### secondary_model

**Type:** String
**Default:** None (disabled)
**Required:** No

A secondary Whisper model that can be triggered on-demand using the `model_modifier` hotkey or the `--model` CLI flag. Useful for having a fast model for everyday use and a more accurate model available when needed.

**Example:**
```toml
[hotkey]
model_modifier = "LEFTSHIFT"

[whisper]
model = "base.en"             # Fast model for everyday use
secondary_model = "large-v3-turbo"  # Accurate model when needed
```

**Usage:**
- Hold `model_modifier` while pressing the hotkey to use the secondary model
- Or use CLI: `voxtype record start --model large-v3-turbo`

### available_models

**Type:** Array of strings
**Default:** `[]`
**Required:** No

Additional models that can be requested via the `--model` CLI flag. The primary `model` and `secondary_model` are always available; this list adds more options.

**Example:**
```toml
[whisper]
model = "base.en"
secondary_model = "large-v3-turbo"
available_models = ["medium.en", "small.en"]  # Additional models for CLI
```

**Usage:**
```bash
voxtype record start --model medium.en
```

**Note:** Models must be downloaded before use. Run `voxtype setup --download --model <name>` to download.

### max_loaded_models

**Type:** Integer
**Default:** `2`
**Required:** No

Maximum number of models to keep loaded in memory simultaneously. When this limit is reached and a new model is requested, the least recently used non-primary model is evicted.

**Example:**
```toml
[whisper]
model = "base.en"
secondary_model = "large-v3-turbo"
max_loaded_models = 3  # Keep up to 3 models in memory
```

**Notes:**
- The primary model is never evicted
- Only applies when `gpu_isolation = false` (subprocess mode doesn't cache models)
- Higher values use more memory but reduce model loading latency

### cold_model_timeout_secs

**Type:** Integer
**Default:** `300` (5 minutes)
**Required:** No

Time in seconds after which idle non-primary models are automatically evicted from memory. Set to `0` to disable auto-eviction.

**Example:**
```toml
[whisper]
model = "base.en"
secondary_model = "large-v3-turbo"
cold_model_timeout_secs = 60  # Evict unused models after 1 minute
```

**Notes:**
- Only evicts models that haven't been used within the timeout period
- The primary model is never evicted
- Helps free memory when switching models infrequently

---

## Remote Backend Settings

The following options are used when `backend = "remote"`. They have no effect when using local transcription.

> **Privacy Notice**: Remote transcription sends your audio over the network. This feature was designed for users who self-host Whisper servers on their own hardware. While it can also connect to cloud services like OpenAI, users with privacy concerns should carefully consider the implications. See [User Manual - Remote Whisper Servers](USER_MANUAL.md#remote-whisper-servers) for details.

### remote_endpoint

**Type:** String
**Default:** None
**Required:** Yes (when `backend = "remote"`)

The base URL of the remote Whisper server. Must include the protocol (`http://` or `https://`).

**Examples:**
```toml
[whisper]
backend = "remote"

# Self-hosted whisper.cpp server
remote_endpoint = "http://192.168.1.100:8080"

# OpenAI API
remote_endpoint = "https://api.openai.com"
```

**Security note:** Voxtype logs a warning if you use HTTP (unencrypted) for non-localhost endpoints, as your audio would be transmitted in the clear.

### remote_model

**Type:** String
**Default:** `"whisper-1"`
**Required:** No

The model name to send to the remote server.

- For **whisper.cpp server**: This is ignored (the server uses whatever model it was started with)
- For **OpenAI API**: Must be `"whisper-1"`
- For **other providers**: Check their documentation

**Example:**
```toml
[whisper]
backend = "remote"
remote_endpoint = "https://api.openai.com"
remote_model = "whisper-1"
```

### remote_api_key

**Type:** String
**Default:** None
**Required:** No (depends on server)

API key for authenticating with the remote server. Sent as a Bearer token in the Authorization header.

**Recommendation:** Use the `VOXTYPE_WHISPER_API_KEY` environment variable instead of putting keys in your config file.

**Example using environment variable:**
```bash
export VOXTYPE_WHISPER_API_KEY="sk-..."
```

**Example in config (less secure):**
```toml
[whisper]
backend = "remote"
remote_endpoint = "https://api.openai.com"
remote_api_key = "sk-..."
```

### remote_timeout_secs

**Type:** Integer
**Default:** `30`
**Required:** No

Maximum time in seconds to wait for the remote server to respond. Increase for slow networks or when transcribing long audio.

**Example:**
```toml
[whisper]
backend = "remote"
remote_endpoint = "http://192.168.1.100:8080"
remote_timeout_secs = 60  # 60 second timeout for long recordings
```

### whisper_cli_path

**Type:** String
**Default:** Auto-detected from PATH
**Required:** No
**Platform:** Linux only

Path to the `whisper-cli` binary. Only used when `backend = "cli"`.

If not specified, voxtype searches for `whisper-cli` or `whisper` in:
1. Your `$PATH`
2. Common system locations (`/usr/local/bin`, `/usr/bin`)
3. Current directory
4. `~/.local/bin`

**Example:**
```toml
[whisper]
backend = "cli"
whisper_cli_path = "/opt/whisper.cpp/build/bin/whisper-cli"
```

**Installing whisper-cli:**

Build from source at [github.com/ggerganov/whisper.cpp](https://github.com/ggerganov/whisper.cpp):

```bash
git clone https://github.com/ggerganov/whisper.cpp
cd whisper.cpp
cmake -B build
cmake --build build --config Release
sudo cp build/bin/whisper-cli /usr/local/bin/
```

---

## [parakeet]

Configuration for the Parakeet speech-to-text engine. This section is only used when `engine = "parakeet"`.

See [PARAKEET.md](PARAKEET.md) for detailed setup instructions.

### model

**Type:** String
**Default:** `"parakeet-tdt-0.6b-v3"`
**Required:** No

The Parakeet model to use. Can be a model name (looked up in `~/.local/share/voxtype/models/`) or an absolute path to a model directory.

**Example:**
```toml
[parakeet]
model = "parakeet-tdt-0.6b-v3"
```

**Using absolute path:**
```toml
[parakeet]
model = "/opt/models/parakeet-tdt-0.6b-v3"
```

### model_type

**Type:** String
**Default:** Auto-detected from model files
**Required:** No

The model architecture type. Usually auto-detected based on files present in the model directory.

**Values:**
- `tdt` - Token-Duration-Transducer (recommended, proper punctuation)
- `ctc` - Connectionist Temporal Classification (faster, character-level)

**Example:**
```toml
[parakeet]
model = "parakeet-tdt-0.6b-v3"
model_type = "tdt"
```

### on_demand_loading

**Type:** Boolean
**Default:** `false`
**Required:** No

Same behavior as `[whisper].on_demand_loading`. When `true`, loads the model only when recording starts and unloads after transcription.

**Example:**
```toml
[parakeet]
model = "parakeet-tdt-0.6b-v3"
on_demand_loading = true  # Free memory when not transcribing
```

### streaming

**Type:** Boolean
**Default:** `false`
**Required:** No

When `true`, voxtype types text incrementally while you are still speaking
instead of waiting for hotkey release. Uses the parakeet-rs cache-aware
streaming pipeline and a TDT v3 family model with `tokenizer.model`.

**Requires toggle activation.** Streaming output types characters at the
cursor while you dictate. On Wayland compositors backed by libinput
(Hyprland, Sway, River), synthetic key events emitted by `wtype` and
`dotool` clobber the held-key state tracker, so the release of a held PTT
key never fires `bindrd` and the daemon gets stuck in streaming. Use
`[hotkey] mode = "toggle"`, or bind your compositor to `voxtype record
toggle` rather than a press/release pair. The daemon auto-promotes
`push_to_talk` to `toggle` at startup when streaming is enabled and emits
a warning to the log.

**Example:**
```toml
engine = "parakeet"

[parakeet]
model = "parakeet-tdt-0.6b-v3"
streaming = true

[hotkey]
mode = "toggle"
```

### Complete Example

```toml
engine = "parakeet"

[parakeet]
model = "parakeet-tdt-0.6b-v3"
on_demand_loading = false  # Keep model loaded for fast response
```

---

## [moonshine]

Configuration for the Moonshine speech-to-text engine. This section is only used when `engine = "moonshine"`.

> **Note:** Moonshine support is experimental. See [MOONSHINE.md](MOONSHINE.md) for detailed setup instructions.

### model

**Type:** String
**Default:** `"base"`
**Required:** No

The Moonshine model to use. Can be a model name (looked up in `~/.local/share/voxtype/models/moonshine-{name}/`) or an absolute path to a model directory.

**Available models:**

| Model | Parameters | Size | Description |
|-------|-----------|------|-------------|
| `tiny` | 27M | 100MB | Fastest, English |
| `base` | 61M | 237MB | Better accuracy, English |
| `base-ja` | 61M | 237MB | Multilingual (Japanese) |
| `base-zh` | 61M | 237MB | Multilingual (Mandarin) |
| `tiny-ja` | 27M | 100MB | Multilingual (Japanese) |
| `tiny-zh` | 27M | 100MB | Multilingual (Mandarin) |
| `tiny-ko` | 27M | 100MB | Multilingual (Korean) |
| `tiny-ar` | 27M | 100MB | Multilingual (Arabic) |

**Example:**
```toml
[moonshine]
model = "base"
```

**Using absolute path:**
```toml
[moonshine]
model = "/opt/models/moonshine-base"
```

### quantized

**Type:** Boolean
**Default:** `true`
**Required:** No

Use quantized model files if available. Quantized models are smaller and faster. Falls back to full precision if quantized files are not found.

**Example:**
```toml
[moonshine]
model = "base"
quantized = true
```

### on_demand_loading

**Type:** Boolean
**Default:** `false`
**Required:** No

Same behavior as `[whisper].on_demand_loading`. When `true`, loads the model only when recording starts and unloads after transcription.

**Example:**
```toml
[moonshine]
model = "base"
on_demand_loading = true  # Free memory when not transcribing
```

### Configuration Summary

| Option | CLI Flag | Environment Variable | Default | Description |
|--------|----------|---------------------|---------|-------------|
| `model` | `--model` | `VOXTYPE_MODEL` | `"base"` | Moonshine model name or path |
| `quantized` | - | - | `true` | Use quantized model files when available |
| `on_demand_loading` | - | - | `false` | Load model only when recording starts |

### Complete Example

```toml
engine = "moonshine"

[moonshine]
model = "base"
quantized = true
on_demand_loading = false  # Keep model loaded for fast response
```

---

## [cohere]

Configuration for the Cohere Transcribe speech-to-text engine. This section is only used when `engine = "cohere"`.

Cohere Transcribe is an encoder-decoder ASR model from Cohere Labs. It currently sits at #1 on the Open ASR Leaderboard. Whisper-style task tokens give it punctuation, capitalization, and inverse text normalization out of the box.

### model

**Type:** String
**Default:** `"cohere-transcribe-int8"`
**Required:** No

The Cohere model to use. Can be a model name (looked up in `~/.local/share/voxtype/models/<name>/`) or an absolute path to a model directory.

**Available models:**

| Model | Quantization | Size | Notes |
|-------|--------------|------|-------|
| `cohere-transcribe-q4f16` | int4 weights, FP16 KV | ~1.5 GB | Recommended; smallest download, fastest CPU |
| `cohere-transcribe-q4` | int4 weights, FP32 KV | ~2.0 GB | Same accuracy as q4f16, larger memory |
| `cohere-transcribe-int8` | int8 | ~2.9 GB | Quality reference for quantized models |
| `cohere-transcribe-fp16` | FP16 | ~3.9 GB | Highest accuracy, largest download |

All variants are HuggingFace Optimum exports of Cohere Transcribe (16384 vocab, 14 languages). Download via `voxtype setup model` (interactive) — pick the Cohere section and confirm the size warning.

**Performance (warm CPU, voxtype 0.7.0, dictation-length audio):**

| Variant | Realtime factor | Notes |
|---------|-----------------|-------|
| q4f16 | 9-11× | Best CPU throughput |
| q4 | 9-11× | Same speed as q4f16 |
| int8 | 2-3× | Slowest CPU path |
| fp16 | 7-8× | |

**GPU acceleration (CUDA):** The `voxtype-onnx-cuda-12` and `voxtype-onnx-cuda-13` binaries register the CUDA execution provider on the encoder. The decoder is pinned to CPU because ORT's CUDA `GroupQueryAttention` kernel does not yet accept the `attention_bias` input that the HF Optimum decoder export uses. Encoder-on-GPU is where weight matmuls dominate, so this hybrid is most of the win.

GPU speedup is hardware- and length-dependent. On a GTX 1660 Ti + i9-9900KF with q4f16:

| Audio length | CPU only | Encoder GPU + Decoder CPU |
|--------------|----------|---------------------------|
| 4.75s | 5.0× realtime | 4.6× realtime |
| 28.5s | 5.9× realtime | 8.2× realtime (~28% faster) |

The fixed CUDA setup cost dominates short clips; longer utterances and faster GPUs (RTX 30/40 series) pull further ahead. Once ORT lands the missing GQA kernel, the decoder will move to the GPU automatically without a config change.

**Example:**
```toml
[cohere]
model = "cohere-transcribe-int8"
```

### language

**Type:** String
**Default:** `"en"`
**Required:** No

Two-letter ISO 639-1 language code. Cohere officially supports 14 languages.

**Supported values:** `ar`, `de`, `en`, `es`, `fr`, `hi`, `it`, `ja`, `ko`, `nl`, `pt`, `ru`, `tr`, `zh`.

**Example:**
```toml
[cohere]
language = "fr"
```

The daemon resolves the language to its decoder prefix at startup. Unsupported codes are rejected with a clear error.

### threads

**Type:** Integer (optional)
**Default:** unset (uses `min(num_cpus, 4)`)
**Required:** No

Number of CPU threads for ONNX Runtime intra-op parallelism. Leave unset on most machines.

**Example:**
```toml
[cohere]
threads = 8
```

### on_demand_loading

**Type:** Boolean
**Default:** `false`
**Required:** No

Same behavior as `[whisper].on_demand_loading`. When `true`, loads the model only when recording starts and unloads after transcription. Useful when working on a laptop where 3 GB of RAM dedicated to the daemon is too costly.

**Example:**
```toml
[cohere]
on_demand_loading = true
```

### Configuration Summary

| Option | CLI Flag | Environment Variable | Default | Description |
|--------|----------|---------------------|---------|-------------|
| `model` | `--model` | `VOXTYPE_MODEL` | `"cohere-transcribe-q4f16"` | Cohere model name or path |
| `language` | `--language` | `VOXTYPE_LANGUAGE` | `"en"` | One of the 14 supported language codes |
| `threads` | - | - | auto | ONNX intra-op thread count |
| `on_demand_loading` | - | - | `false` | Load model only when recording starts |

### Complete Example

```toml
engine = "cohere"

[cohere]
model = "cohere-transcribe-q4f16"
language = "en"
on_demand_loading = false
```

### Building from Source

Source builds need the `cohere` Cargo feature. Optional GPU acceleration via `cohere-cuda` or `cohere-tensorrt`:

```bash
cargo build --release --features cohere           # CPU
cargo build --release --features cohere-cuda      # NVIDIA GPU
cargo build --release --features cohere-tensorrt  # NVIDIA + TensorRT EP
```

The prebuilt `voxtype-*-onnx-*` release binaries already include `cohere`, so users installing via AUR/.deb/.rpm don't need to rebuild.

---

## [soniox]

Configuration for the Soniox cloud streaming WebSocket STT engine. This section is only used when `engine = "soniox"`.

Soniox is a paid cloud STT provider with 60+ languages, per-token finality flags, and server-side endpoint detection. Unlike voxtype's other engines, no model runs on your machine — audio streams to Soniox's servers over WebSocket and tokens stream back.

**Privacy:** Audio is sent to a third-party service. Use the local engines (Whisper, Parakeet, etc.) if you cannot send dictation off-device.

### api_key

**Type:** String (optional)
**Default:** unset (falls back to `SONIOX_API_KEY` env var)
**Required:** Yes (via this field or env var)

Soniox API key. Get one at https://console.soniox.com.

Prefer the env var so the key never lands in shell history or a checked-in config file:

```bash
export SONIOX_API_KEY="your-key-here"
```

### model

**Type:** String
**Default:** `"stt-rt-v4"`
**Required:** No

Soniox model identifier. The current realtime model is `stt-rt-v4`.

### language_hints

**Type:** Array of strings
**Default:** `["hu", "en"]`
**Required:** No

ISO 639-1 codes hinting which languages to prefer. Use an empty array for full auto-detect across all 60+ supported languages.

```toml
[soniox]
language_hints = ["en"]            # English only
# or
language_hints = ["hu", "en", "de"] # Hungarian, English, German
# or
language_hints = []                 # auto-detect everything
```

### language_hints_strict

**Type:** Boolean
**Default:** `true`
**Required:** No

When `true`, the model is strongly biased to produce output only in the languages listed in `language_hints`. When `false`, the model may occasionally produce other languages it detects with high confidence. Ignored when `language_hints` is empty.

Strict mode is the right default for bilingual setups (`["hu", "en"]` etc.): without it, partials can briefly drift to a third language before snapping back when a final lands, causing unnecessary tail revisions. Turn it off only when you genuinely expect input in languages outside the hint list. See [Soniox language-restrictions docs](https://soniox.com/docs/stt/concepts/language-restrictions).

```toml
[soniox]
language_hints = ["hu", "en"]
language_hints_strict = false   # allow occasional third-language tokens
```

### streaming

**Type:** Boolean
**Default:** `true`
**Required:** No

Activation mode for the Soniox backend:

- `true` — Live WebSocket session. Tokens stream back during recording and are typed at the cursor as they arrive (or only on finalization if `type_partials = false`). **Requires `[hotkey] mode = "toggle"`.** Push-to-talk is auto-promoted to toggle for the running session with a warning, because typing characters while the PTT key is still held clobbers libinput's held-key state on Hyprland/Sway/River.
- `false` — Batch mode. Audio buffered while the hotkey is held; on release one WebSocket session opens, the entire buffer is sent + finalized, and the resulting transcript is typed in one shot. Push-to-talk compatible. Loses live partials but keeps Soniox's accuracy.

### type_partials

**Type:** Boolean
**Default:** `true`
**Required:** No

Only used when `streaming = true`. When `true`, non-final tokens are typed at the cursor as they arrive (lower perceived latency). When `false`, only finalized segments are typed — partials still appear in `voxtype status --follow` but never touch the cursor.

Soniox guarantees stable finals; non-finals can be revised. In practice revisions are rare and short. If you see occasional churn at the cursor, set `type_partials = false`.

### context

**Type:** String (optional)
**Default:** unset
**Required:** No

Free-form domain context. Mapped to `context.text` in Soniox's init frame. Use for short prose describing the dictation domain — `"medical consultation"`, `"Rust async runtime podcast"`. Leave unset unless you have a clearly bounded vocabulary worth biasing the model toward. See [Soniox context docs](https://soniox.com/docs/stt/concepts/context).

### terms

**Type:** Array of strings (optional)
**Default:** unset
**Required:** No

Inline vocabulary boost terms. Mapped to `context.terms` in Soniox's init frame. Use for proper names, jargon, product names — entries the generic model wouldn't get right. Combined with `terms_file` (deduplicated, order preserved).

```toml
[soniox]
terms = ["Voxtype", "Hyprland", "tokio-tungstenite"]
```

### terms_file

**Type:** Path (optional)
**Default:** unset
**Required:** No

Path to a JSON file containing a list of vocabulary boost terms — `["term1", "term2", ...]`. Loaded once at daemon startup and merged into `context.terms`. Useful for sharing a corrections list across multiple voxtype config snapshots or projects.

```toml
[soniox]
terms_file = "/home/me/dotfiles/voxtype-terms.json"
```

### async_api

**Type:** Boolean
**Default:** `false`
**Required:** No

Use the Soniox **async transcription API** (file upload + poll) instead of the realtime WebSocket. Different model (`stt-async-v4`), different accuracy profile, batch only — no live partials, no flicker, push-to-talk compatible.

When `true`:
- Audio buffered while recording. On release, voxtype uploads the WAV to `https://api.soniox.com/v1/files`, creates a transcription job, polls until complete, fetches the transcript, then types it at the cursor in one shot.
- `streaming` and `type_partials` are ignored.
- `model` defaults to `stt-async-v4` (override only if you know what you're doing).
- Push-to-talk is **not** auto-promoted to toggle (no live cursor typing means no compositor-state clobbering).

Latency: typical 15s recording → ~1s upload + 2-5s processing = 3-6s total wait after release. Compare to realtime which streams partials as you speak.

**Accuracy:** the async model (`stt-async-v4`) is marketed as higher accuracy than the realtime model (`stt-rt-v4`). In practice quality varies by language and content — benchmark both for your use case before committing.

### async_max_wait_secs

**Type:** Integer
**Default:** `120`
**Required:** No

Maximum total wait time (seconds) for an async API job to complete. If exceeded, voxtype cleans up the server-side job and surfaces an error. Only used when `async_api = true`.

### Configuration Summary

| Option | CLI Flag | Environment Variable | Default | Description |
|--------|----------|---------------------|---------|-------------|
| `api_key` | `--soniox-api-key` | `SONIOX_API_KEY` | none (required) | Soniox API key |
| `model` | - | - | `"stt-rt-v4"` | Soniox model (`stt-async-v4` when `async_api = true`) |
| `language_hints` | - | - | `["hu", "en"]` | Language preference |
| `language_hints_strict` | - | - | `true` | Restrict output to hinted languages (ignored if empty) |
| `streaming` | - | - | `true` | Live WebSocket vs batch-on-release (realtime only) |
| `type_partials` | - | - | `true` | Type non-final tokens at cursor (realtime only) |
| `context` | - | - | none | Free-form domain context (`context.text`) |
| `terms` | - | - | none | Inline boost terms array (`context.terms`) |
| `terms_file` | - | - | none | JSON file path with boost terms |
| `async_api` | - | - | `false` | Use async REST API instead of realtime WS |
| `async_max_wait_secs` | - | - | `120` | Async job total timeout |

### Complete Example — Realtime (with live partials)

```toml
engine = "soniox"

[hotkey]
mode = "toggle"   # Required when [soniox] streaming = true

[soniox]
language_hints = ["hu", "en"]
streaming = true
type_partials = true
# api_key set via SONIOX_API_KEY env var
```

### Complete Example — Async (PTT-compatible, batch-only)

```toml
engine = "soniox"

[hotkey]
mode = "push_to_talk"   # Works with async_api; no toggle promotion

[soniox]
async_api = true
language_hints = ["hu", "en"]
# model defaults to stt-async-v4 when async_api = true
# api_key set via SONIOX_API_KEY env var
```

### Building from Source

Source builds need the `soniox` Cargo feature:

```bash
cargo build --release --features soniox
```

The `soniox` feature is independent of the other engine features and adds a small WebSocket client (tokio-tungstenite + rustls) plus an async HTTP client (reqwest) for the async API. It can be combined with any local engine feature, e.g. `--features "soniox parakeet"` for a binary that runs Parakeet locally and Soniox in the cloud depending on the `engine` setting.

---

## [output]

Controls how transcribed text is delivered.

### mode

**Type:** String
**Default:** `"type"`
**Required:** No

Primary output method.

**Values:**
- `type` - Simulate keyboard input at cursor position (uses wtype, dotool, or ydotool)
- `clipboard` - Copy text to clipboard (requires wl-copy)
- `paste` - Copy to clipboard then simulate paste keystroke (requires wl-copy, and wtype, dotool, or ydotool)
- `file` - Write transcription to a file (requires `file_path` to be set)

**Example:**
```toml
[output]
mode = "paste"
```

**Example (file output):**
```toml
[output]
mode = "file"
file_path = "~/transcriptions/output.txt"
file_mode = "append"
```

**Note about wtype compatibility:**
wtype does not work on KDE Plasma or GNOME Wayland because these compositors don't support the virtual keyboard protocol. On these desktops, voxtype automatically falls back to dotool (if installed) or ydotool. For ydotool, the daemon must be running (`systemctl --user enable --now ydotool`). See [Troubleshooting](TROUBLESHOOTING.md#wtype-not-working-on-kde-plasma-or-gnome-wayland) for details.

**Note about non-US keyboard layouts:**
For non-US keyboard layouts (German QWERTZ, French AZERTY, etc.), dotool is recommended over ydotool. Set `dotool_xkb_layout` to your layout code (e.g., `"de"` for German) when using direct dotool fallback. ydotool does not support keyboard layouts and will produce incorrect characters (e.g., 'y' and 'z' swapped on German layouts).

For multilingual dictation, prefer `language_to_layout` and
`language_to_variant` so voxtype can use the right keymap for each
transcription through direct dotool fallback. `dotoolc` does not work with
voxtype's variants. When using dotool, you must also switch the active desktop
keyboard layout to the language/variant you want to type in.

**Note about paste mode:**
The `paste` mode is an alternative for non-US keyboard layouts. Instead of typing characters directly, it copies text to the clipboard and simulates a paste keystroke. This works regardless of keyboard layout but overwrites your clipboard. Requires wl-copy for clipboard access.

### paste_keys

**Type:** String
**Default:** `"ctrl+v"`
**Required:** No

Keystroke to simulate for paste mode. Change this if your environment uses a different paste shortcut.

**Format:** `"modifier+key"` or `"modifier+modifier+key"` (case-insensitive)

**Common values:**
- `"ctrl+v"` - Standard paste (default)
- `"shift+insert"` - Universal paste for Hyprland/Omarchy
- `"ctrl+shift+v"` - Some terminal emulators

**Example:**
```toml
[output]
mode = "paste"
paste_keys = "shift+insert"  # For Hyprland/Omarchy
```

**Supported keys:**
- Modifiers: `ctrl`, `shift`, `alt`, `super` (also `leftctrl`, `rightctrl`, etc.)
- Letters: `a-z`
- Special: `insert`, `enter`

### restore_clipboard

**Type:** Boolean
**Default:** `false`
**Required:** No
**Applies to:** Paste mode only

When `true`, voxtype saves your clipboard content before transcription and restores it after the paste operation completes. This prevents your original clipboard content from being overwritten by the transcription.

**How it works:**
1. Before transcription: Save current clipboard content (including MIME type for binary data)
2. Copy transcribed text to clipboard
3. Simulate paste keystroke
4. After brief delay: Restore original clipboard content

**Example:**
```toml
[output]
mode = "paste"
restore_clipboard = true  # Preserve original clipboard content
```

**Note:** This only works in `mode = "paste"`. In `mode = "clipboard"`, the user manually pastes the content, so restoration would interfere with the intended workflow.

### restore_clipboard_delay_ms

**Type:** Integer
**Default:** `200`
**Required:** No
**Applies to:** Paste mode only (when `restore_clipboard = true`)

Delay in milliseconds after the paste keystroke before restoring the original clipboard content. Increase this if the restoration happens too quickly and interferes with the paste operation. The default of 200ms works well for most applications including Electron apps (Slack, Discord, VS Code).

**Example:**
```toml
[output]
mode = "paste"
restore_clipboard = true
restore_clipboard_delay_ms = 300  # Longer delay for slower systems
```

### fallback_to_clipboard

**Type:** Boolean
**Default:** `true`
**Required:** No

When `true` and `mode = "type"`, falls back to clipboard if typing fails.

**Note:** This setting is ignored when `driver_order` is set, since the driver list explicitly defines what's tried.

**Example:**
```toml
[output]
mode = "type"
fallback_to_clipboard = true  # Use clipboard if typing drivers fail
```

### driver_order

**Type:** Array of strings
**Default:** `["wtype", "eitype", "dotool", "ydotool", "clipboard", "xclip"]`
**Required:** No

Custom order of output drivers to try when `mode = "type"`. Each driver is tried in sequence until one succeeds. This allows you to prefer specific drivers or exclude others entirely.

**Available drivers:**
- `wtype` - Wayland virtual keyboard protocol (best CJK/Unicode support, wlroots compositors only)
- `eitype` - Wayland via libei/EI protocol (works on GNOME, KDE, and compositors with libei support). On KDE Plasma 6, each invocation briefly registers via the XDG RemoteDesktop portal, which can cause a system-tray icon to flicker during streaming dictation (many fast typing calls). Prefer `dotool` for streaming if you're on KDE.
- `dotool` - uinput-based typing (supports keyboard layouts, works on X11/Wayland/TTY). For streaming backends (Parakeet, Soniox), run `dotoold` to make this **much** faster when no per-call layout or variant hint is needed — see [Streaming performance: dotoold fast path](#streaming-performance-dotoold-fast-path) below.
- `ydotool` - uinput-based typing (requires `ydotoold` daemon, X11/Wayland/TTY). Fast spawn, but **does not support keyboard layouts** — sends raw US keycodes. Wrong output on non-US layouts (e.g. Hungarian Z/Y swap).
- `clipboard` - Wayland clipboard via wl-copy
- `xclip` - X11 clipboard via xclip

**Default behavior (no driver_order set):**
The default chain is: wtype → eitype → dotool → ydotool → clipboard → xclip

**Examples:**

```toml
[output]
mode = "type"

# Prefer ydotool over dotool, skip wtype
driver_order = ["ydotool", "dotool", "clipboard"]

# X11-only setup
driver_order = ["dotool", "ydotool", "xclip"]

# Force single driver (no fallback)
driver_order = ["ydotool"]

# GNOME/KDE Wayland (prefer eitype, wtype doesn't work)
driver_order = ["eitype", "dotool", "clipboard"]
```

**CLI override:**
```bash
voxtype --driver=ydotool,clipboard daemon
```

**Note:** When `driver_order` is set, `fallback_to_clipboard` is ignored—the driver list explicitly defines what's tried.

#### Streaming performance: dotoold fast path

Streaming backends (Parakeet, Soniox) call the output driver many times per session — once for every partial token batch. With direct `dotool` invocations each call spawns a fresh dotool process that pays the kernel uinput device setup cost (**~700-800ms** on most systems). For 60+ partials per session this stacks into 40+ seconds of typing latency — unusable.

dotool ships a daemon/client pair (`dotoold` + `dotoolc`) specifically for this case. When `dotoold` is running and voxtype has no per-call XKB layout or variant hint, voxtype auto-detects its FIFO at `/tmp/dotool-pipe` and routes typing through `dotoolc`, which simply relays commands to the long-lived daemon. The uinput device is registered **once** at daemon startup, not on every typed segment. Sub-10ms per call.

**Strongly recommended** if you use `dotool` as your primary typing driver with any streaming backend.

**Setup as a systemd user unit (persistent across reboots):**

```bash
mkdir -p ~/.config/systemd/user

cat > ~/.config/systemd/user/dotoold.service <<'EOF'
[Unit]
Description=dotool daemon for low-latency keyboard injection
After=default.target

[Service]
ExecStart=/usr/bin/dotoold
# Set DOTOOL_XKB_LAYOUT here when you want the dotoold fast path with one
# fixed dotool keymap. dotoolc does not work with variants and cannot receive
# voxtype's per-call XKB hints, so voxtype uses direct dotool instead whenever
# it needs a layout or variant hint.
Environment=DOTOOL_XKB_LAYOUT=hu
Restart=on-failure

[Install]
WantedBy=default.target
EOF

systemctl --user enable --now dotoold
```

Replace `hu` with your XKB layout (`de`, `fr`, `us`, etc.).

**Manual test:**
```bash
DOTOOL_XKB_LAYOUT=hu dotoold &
ls -la /tmp/dotool-pipe   # confirm FIFO exists
```

**Verifying voxtype is using the fast path:**

After dictating a session, check the daemon log:
```bash
journalctl --user -u voxtype --since "5 min ago" | grep "typed via"
```
- `Text typed via dotoolc (N chars)` — fast path active
- `Text typed via dotool (N chars)` — direct path; either the daemon is not running or voxtype had a per-call XKB hint

The layout setting for the fast path applies to **the daemon, not the client**.
If you need the `dotoold` fast path with one fixed layout, set
`DOTOOL_XKB_LAYOUT` in dotoold's unit file or shell and leave voxtype's dotool
XKB fields unset.

`dotoolc` does not work with variants and cannot receive voxtype's per-call XKB
hints. When voxtype has a per-call XKB layout or variant hint, it bypasses
`dotoolc` and invokes direct `dotool` so the hint is used for text-to-key
lookup.

**Important direct-dotool caveat:** direct dotool's keyboard layout
(`DOTOOL_XKB_LAYOUT` / `DOTOOL_XKB_VARIANT`) only controls how dotool converts
text to key events. It does **not** switch the active desktop/compositor layout.
If the focused app is still using an English layout, Russian phonetic key events
will be interpreted as English letters. Switch your desktop layout to the target
layout/variant before dictating.

### dotool_xkb_layout

**Type:** String (optional)
**Default:** None
**Required:** No

Keyboard layout for direct dotool fallback. Required for non-US keyboard
layouts (German, French, etc.) when using dotool as the typing backend.

dotool is automatically used as a fallback when wtype fails (e.g., on GNOME/KDE Wayland). Unlike ydotool, direct dotool fallback supports keyboard layouts via XKB environment variables.

This setting tells direct `dotool` which XKB keymap to use when converting
Unicode text to physical key events. Setting it in voxtype makes voxtype use
direct `dotool` instead of the `dotoolc` fast path, because `dotoolc` does not
work with variants and cannot receive voxtype's per-call XKB hints.

It does not change the active desktop layout. Before dictating with dotool,
switch your desktop/compositor to the same layout.

**Common values:**
- `"de"` - German (QWERTZ)
- `"fr"` - French (AZERTY)
- `"es"` - Spanish
- `"uk"` - Ukrainian
- `"ru"` - Russian

**Example:**
```toml
[output]
mode = "type"
dotool_xkb_layout = "de"  # German keyboard layout
```

### dotool_xkb_variant

**Type:** String (optional)
**Default:** None
**Required:** No

Keyboard layout variant for direct dotool fallback. Use this for layout
variations like `nodeadkeys`.

`dotoolc` does not work with variants. Setting this in voxtype makes
voxtype use direct `dotool` so the variant can be passed to that invocation.
As with `dotool_xkb_layout`, this configures dotool's key lookup only. The
active desktop layout must already be using the same variant.

**Example:**
```toml
[output]
dotool_xkb_layout = "de"
dotool_xkb_variant = "nodeadkeys"  # German without dead keys
```

### eitype_xkb_layout

**Type:** String (optional)
**Default:** None
**Required:** No

Keyboard layout passed to eitype as `-l <layout>`. Use this when your
transcribed language does not match the active system layout (issue #180).

When unset, voxtype derives the layout from the transcriber's detected
language using [language_to_layout](#language_to_layout). Setting this field
explicitly disables that auto-detection and forces the chosen layout.

**Example: pin eitype to US regardless of what voxtype detects**
```toml
[output]
mode = "type"
driver_order = ["eitype"]
eitype_xkb_layout = "us"
```

### eitype_xkb_variant

**Type:** String (optional)
**Default:** None
**Required:** No

Layout variant passed to eitype as `--variant <variant>` (e.g., `dvorak`,
`colemak`, `nodeadkeys`).

```toml
[output]
eitype_xkb_layout = "de"
eitype_xkb_variant = "nodeadkeys"
```

### language_to_layout

**Type:** Table (map of two-letter language code to XKB layout)
**Default:** Built-in map covering common languages

Maps detected language codes (ISO 639-1, e.g. `en`, `ru`, `de`) to XKB
keyboard layout codes (e.g. `us`, `ru`, `de`). When a transcriber reports the
language used for a transcription and neither `eitype_xkb_layout` nor
`dotool_xkb_layout` is set, voxtype looks the language up in this map and
passes the resulting layout hint to eitype/dotool for that transcription.

This is what makes the multi-language case from issue #180 work
end-to-end: with `language = ["en", "ru"]` and `driver_order = ["eitype"]`,
voxtype detects the spoken language, looks it up here, and tells eitype
to type with the right keyboard layout.

For dotool, this map chooses the keymap used by direct dotool fallback to
convert text into key events. `dotoolc` does not work with variants and cannot
receive voxtype's per-call XKB hints, so voxtype bypasses `dotoolc` for those
calls. This does not switch the active desktop layout. If you use dotool for
Russian phonetic typing, switch your desktop layout to Russian phonetic before
dictating Russian.

**Built-in defaults include:**
- `en = "us"` (English)
- `ru = "ru"`, `de = "de"`, `fr = "fr"`, `es = "es"`, `it = "it"`
- `pl = "pl"`, `uk = "uk"`, `cs = "cs"`, `sk = "sk"`
- `sv = "sv"`, `no = "no"`, `fi = "fi"`, `da = "da"`, `nl = "nl"`
- `pt = "pt"`, `tr = "tr"`, `gr = "gr"`, `hu = "hu"`, `ro = "ro"`
- `bg = "bg"`, `hr = "hr"`, `sr = "sr"`, `sl = "sl"`
- `lt = "lt"`, `lv = "lv"`, `et = "et"`, `is = "is"`
- `ca = "ca"`, `eu = "eu"`
- `el = "gr"` (Greek uses "gr")
- `ja = "jp"`, `ko = "kr"`

Languages without a mapping fall through with no layout hint (eitype uses
the system layout).

**Replacing the defaults.** Providing the `[output.language_to_layout]`
section in your config replaces the entire built-in map (TOML does not
merge tables). If you want to add a single entry while keeping the
defaults, copy the entries you need.

**Example: Brazilian Portuguese and Dvorak English**
```toml
[output.language_to_layout]
en = "dvorak"   # English on Dvorak
pt = "br"       # Brazilian Portuguese layout
ru = "ru"       # Russian (kept from defaults)
de = "de"       # German (kept from defaults)
```

**Disabling auto layout selection.** Set the map to empty to skip layout
inference entirely; eitype/dotool will use whatever explicit
`*_xkb_layout` you set (or the system layout):
```toml
[output.language_to_layout]
# (empty)
```

### language_to_variant

**Type:** Table (map of two-letter language code to XKB layout variant)
**Default:** Empty

Maps detected language codes to XKB layout variants for that language. Use this
when a language needs a variant, but that variant must not apply to every
language you dictate.

This is useful for Russian phonetic typing:

```toml
[whisper]
language = ["en", "ru"]

[output.language_to_layout]
en = "us"
ru = "ru"

[output.language_to_variant]
ru = "phonetic"
```

When Russian is detected, voxtype passes `ru` plus `phonetic` to eitype or
direct dotool fallback. When English is detected, it passes `us` with no
variant. `dotoolc` does not work with variants and cannot receive voxtype's
per-call XKB hints, so voxtype bypasses it for these calls. With dotool, also
switch the active desktop layout before dictating; otherwise the focused app
will interpret the key events using whatever layout is currently active.

Explicit driver settings still win. For example, `dotool_xkb_variant =
"nodeadkeys"` prevents `language_to_variant` from changing dotool's key lookup
variant, but eitype can still use the per-language variant if
`eitype_xkb_variant` is unset.

### file_path

**Type:** String (path)
**Default:** None
**Required:** Only when `mode = "file"`

File path for file output mode. When `mode = "file"`, transcriptions are written to this file instead of being typed or copied to clipboard.

This path is also used as the default for the `--output-file` CLI flag when appending.

**Example:**
```toml
[output]
mode = "file"
file_path = "~/transcriptions/output.txt"
```

**Note:** Parent directories are created automatically if they don't exist.

### file_mode

**Type:** String
**Default:** `"overwrite"`
**Required:** No

Controls how file output handles existing files.

**Values:**
- `overwrite` - Replace the file contents on each transcription (default)
- `append` - Add transcription to the end of the file

This setting applies to both config-based file output (`mode = "file"`) and the `--output-file` CLI flag.

**Example:**
```toml
[output]
mode = "file"
file_path = "~/transcriptions/log.txt"
file_mode = "append"  # Build a running log of transcriptions
```

---

## [output.notification]

Controls desktop notifications at various stages.

### on_recording_start

**Type:** Boolean
**Default:** `false`
**Required:** No

When `true`, shows a notification when recording starts (hotkey pressed).

### on_recording_stop

**Type:** Boolean
**Default:** `false`
**Required:** No

When `true`, shows a notification when recording stops (transcription begins).

### on_transcription

**Type:** Boolean
**Default:** `true`
**Required:** No

When `true`, shows a notification with the transcribed text after transcription completes.

**Requires:** `notify-send` (libnotify)

**Example:**
```toml
[output.notification]
on_recording_start = true   # Notify when PTT activates
on_recording_stop = true    # Notify when transcribing
on_transcription = true     # Show transcribed text
```

### urgency

**Type:** String (`"low"`, `"normal"`, or `"critical"`)
**Default:** `"normal"`
**Required:** No

Sets the urgency level passed to `notify-send` for all voxtype notifications.

On GNOME, notifications with `"low"` urgency are delivered to the notification drawer without showing as a popup banner. Use `"normal"` (the default) if you want notifications to pop up on screen. Use `"critical"` if you want notifications that persist until dismissed.

Unknown values fall back to `"normal"`.

**Example:**
```toml
[output.notification]
urgency = "normal"  # "low" | "normal" | "critical"
```

### type_delay_ms

**Type:** Integer
**Default:** `0`
**Required:** No

Delay in milliseconds between each typed character. Increase if characters are being dropped.

**Example:**
```toml
[output]
type_delay_ms = 10  # 10ms delay between characters
```

### pre_type_delay_ms

**Type:** Integer
**Default:** `0`
**Required:** No

Delay in milliseconds before typing starts. This allows the virtual keyboard to initialize and helps prevent the first character from being dropped on some compositors. Try 100-200ms if you experience issues.

> **Note:** When using compositor integration (via `voxtype setup compositor`), best results come from not binding Escape in the submap. Some users have had success with Escape bound by increasing this delay, but the most consistent fix is to use F12 or another key instead.

**Example:**
```toml
[output]
pre_type_delay_ms = 100  # 100ms delay before typing starts
```

### auto_submit

**Type:** Boolean
**Default:** `false`
**Required:** No

Automatically send an Enter keypress after outputting the transcribed text. Useful for chat applications, command lines, or forms where you want to auto-submit after dictation.

**Example:**
```toml
[output]
auto_submit = true  # Press Enter after transcription
```

**Note:** This works with all output modes (`type`, `paste`) but has no effect in `clipboard` mode since clipboard-only output doesn't simulate keypresses.

### append_text

**Type:** String
**Default:** None (disabled)
**Required:** No
**Environment Variable:** `VOXTYPE_APPEND_TEXT`

Text to append after each transcription. Appended after the main transcription but before `auto_submit` (if enabled). Useful for separating sentences when dictating paragraphs incrementally.

**Common use case:** When transcribing a paragraph sentence by sentence, there are no spaces between each sentence. Setting `append_text = " "` adds a space after each transcription, creating proper sentence separation.

**Example:**
```toml
[output]
append_text = " "  # Add a space after each transcription
```

**How it works:**
- In `type` mode: Types the text after the main transcription
- In `paste` mode: Includes the text in the clipboard before pasting
- In `clipboard` mode: Includes the text in the clipboard
- With `auto_submit = true`: The append_text is typed/pasted first, then Enter is sent

**Note:** You can append any text, not just spaces. For example, `append_text = "\n"` would add a newline after each transcription.

### shift_enter_newlines

**Type:** Boolean
**Default:** `false`
**Required:** No

Convert newlines in transcribed text to Shift+Enter instead of regular Enter. This is useful for applications where pressing Enter submits the message or form, but you want to insert line breaks within your text.

**Why use it:**

Many chat and messaging applications (Slack, Discord, Teams, etc.) and some IDEs (Cursor AI chat) use Enter to submit/send and Shift+Enter to insert a line break. When dictating multi-line text, regular newlines would submit prematurely. This option ensures line breaks are inserted without triggering submission.

**Example:**
```toml
[output]
shift_enter_newlines = true  # Use Shift+Enter for newlines
```

**Common use cases:**
- Slack, Discord, Microsoft Teams chat
- AI coding assistants (Cursor, GitHub Copilot Chat)
- Web forms where Enter submits
- Any application where Enter has special meaning

**Note:** This only affects the wtype output driver. When combined with `auto_submit = true`, the final Enter (to submit) is still sent as a regular Enter after all Shift+Enter line breaks.

### wtype_shift_prefix

**Type:** Boolean
**Default:** `false`
**Required:** No
**Environment Variable:** `VOXTYPE_WTYPE_SHIFT_PREFIX`

Prefix wtype output with a Shift key press and release. This is a workaround for apps (notably Discord) that drop the first CJK character when wtype types text. The Shift press/release has no visible effect on the output but prevents the first character from being swallowed.

Only affects the wtype output driver. Has no effect when using dotool, ydotool, or clipboard modes.

**Example:**
```toml
[output]
wtype_shift_prefix = true
```

**CLI override:**
```bash
voxtype --wtype-shift-prefix daemon
```

**When to use:**
- First CJK (Chinese, Japanese, Korean) character is missing from output
- You're using the wtype driver (default on wlroots compositors)
- The problem happens in specific apps like Discord

See [Troubleshooting](TROUBLESHOOTING.md#first-cjk-character-dropped-wtype) for more details.

### pre_output_command

**Type:** String
**Default:** None (disabled)
**Required:** No

Shell command to execute immediately before typing output. Runs after post-processing but before text is typed/pasted.

**Primary use case:** Compositor integration to block modifier keys during typing. When using compositor keybindings with modifiers (e.g., `SUPER+CTRL+X`), if you release keys slowly, held modifiers can interfere with typed output.

**Example:**
```toml
[output]
pre_output_command = "hyprctl dispatch submap voxtype_suppress"
```

**Automatic setup:** Use `voxtype setup compositor hyprland|sway|river` to automatically configure this.

### post_output_command

**Type:** String
**Default:** None (disabled)
**Required:** No

Shell command to execute immediately after typing output completes.

**Primary use case:** Compositor integration to restore normal modifier behavior after typing.

**Example:**
```toml
[output]
post_output_command = "hyprctl dispatch submap reset"
```

**Compositor integration example:**
```toml
[output]
# Switch to modifier-blocking submap before typing
pre_output_command = "hyprctl dispatch submap voxtype_suppress"
# Return to normal submap after typing
post_output_command = "hyprctl dispatch submap reset"
```

**Other uses:**
```toml
[output]
# Notification when typing starts/finishes
pre_output_command = "notify-send 'Typing...'"
post_output_command = "notify-send 'Done'"

# Logging
post_output_command = "echo $(date) >> ~/voxtype.log"
```

See [User Manual - Output Hooks](USER_MANUAL.md#output-hooks-compositor-integration) for detailed setup instructions.

---

## [output.post_process]

Optional post-processing command that runs after transcription. The command receives
the transcribed text on stdin and should output the processed text on stdout.

**Best use cases:**
- **Translation**: Speak in one language, output in another
- **Domain vocabulary**: Medical, legal, or technical term correction
- **Reformatting**: Convert casual dictation to formal prose
- **Filler word removal**: Remove "um", "uh", "like" that Whisper sometimes keeps
- **Custom workflows**: Multi-output scenarios (e.g., translate to 5 languages, save JSON to file, inject only English at cursor)

**Important notes:**
- Adds 2-5 seconds latency depending on model size
- For most users, Whisper large-v3-turbo with Voxtype's built-in `spoken_punctuation` is sufficient
- LLMs interpret text literally—saying "slash" won't produce "/" (use `spoken_punctuation` for that)
- Use **instruct/chat models**, not reasoning models (they output `<think>` blocks)
- Avoid emojis in LLM output—ydotool cannot type them

### command

**Type:** String
**Default:** None (disabled)
**Required:** Yes (if section is present)

The shell command to execute. Text is piped to stdin, processed text read from stdout.

**Examples:**
```toml
# Use Ollama with a small model for quick cleanup
[output.post_process]
command = "ollama run llama3.2:1b 'Clean up this transcription. Fix grammar and remove filler words. Output only the cleaned text:'"

# Simple filler word removal with sed
[output.post_process]
command = "sed 's/\\bum\\b//g; s/\\buh\\b//g; s/\\blike\\b//g'"

# Custom Python script
[output.post_process]
command = "python3 ~/.config/voxtype/cleanup.py"

# LM Studio API (OpenAI-compatible)
[output.post_process]
command = "~/.config/voxtype/lm-studio-cleanup.sh"
```

### timeout_ms

**Type:** Integer
**Default:** `30000` (30 seconds)
**Required:** No

Maximum time in milliseconds to wait for the command to complete. If exceeded,
the original text is used and a warning is logged.

**Recommendations:**
- Simple shell commands: `5000` (5 seconds)
- Local LLMs: `30000-60000` (30-60 seconds)
- Remote APIs: `30000` or higher

**Example:**
```toml
[output.post_process]
command = "ollama run llama3.2:1b 'Clean up:'"
timeout_ms = 45000  # 45 second timeout for LLM
```

### Context from Previous Dictation

When post-processing is enabled, voxtype passes the previous dictation's text via the `VOXTYPE_CONTEXT` environment variable (if the previous dictation was within 60 seconds). This helps LLM-based cleanup scripts maintain continuity across rapid-fire dictations.

- Stdin always contains only the current text (existing scripts work unchanged)
- Scripts that want context can optionally read `$VOXTYPE_CONTEXT`
- In meeting mode, context is tracked per audio source (mic/loopback) to prevent speaker bleed

See the example scripts in `examples/` for how to use `VOXTYPE_CONTEXT`.

### Error Handling

If the post-processing command fails for any reason (command not found, non-zero
exit, timeout, empty output), Voxtype gracefully falls back to the original
transcribed text and logs a warning. This ensures voice-to-text output is never
blocked by post-processing issues.

**Debugging:**
Run voxtype with `-v` or `-vv` to see detailed logs about post-processing:
```bash
voxtype -vv
```

### Example LM Studio Script

For users running LM Studio locally, here's an example script:

```bash
#!/bin/bash
# ~/.config/voxtype/lm-studio-cleanup.sh

INPUT=$(cat)

curl -s http://localhost:1234/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d "{
    \"messages\": [{
      \"role\": \"system\",
      \"content\": \"Clean up this dictated text. Fix spelling, remove filler words (um, uh), add proper punctuation. Output ONLY the cleaned text - no quotes, no emojis, no explanations.\"
    },{
      \"role\": \"user\",
      \"content\": \"$INPUT\"
    }],
    \"temperature\": 0.1
  }" | jq -r '.choices[0].message.content'
```

Make it executable: `chmod +x ~/.config/voxtype/lm-studio-cleanup.sh`

---

## [profiles.*]

Named profiles for context-specific settings. Profiles allow you to define different post-processing commands and output modes for different use cases, selectable at recording time via `--profile`.

### Defining Profiles

Each profile is a TOML table under `[profiles]`:

```toml
[profiles.slack]
post_process_command = "ollama run llama3.2:1b 'Format for Slack:'"

[profiles.code]
post_process_command = "ollama run llama3.2:1b 'Format as code comment:'"
output_mode = "clipboard"

[profiles.email]
post_process_command = "ollama run llama3.2:1b 'Format as professional email:'"
post_process_timeout_ms = 45000
```

### Profile Options

#### post_process_command

**Type:** String
**Default:** None (uses `[output.post_process].command`)
**Required:** No

Shell command for post-processing. Overrides the default `[output.post_process].command` when this profile is active.

#### post_process_timeout_ms

**Type:** Integer
**Default:** None (uses `[output.post_process].timeout_ms` or 30000)
**Required:** No

Timeout in milliseconds for the post-processing command.

#### output_mode

**Type:** String
**Default:** None (uses `[output].mode`)
**Required:** No

Output mode override. Valid values: `type`, `clipboard`, `paste`.

### Using Profiles

Specify a profile when starting a recording:

```bash
voxtype record start --profile slack
voxtype record toggle --profile code
```

### Behavior

- Options not specified in a profile inherit from the main config
- Unknown profile names log a warning and use default settings
- Profiles have no effect on `record stop` or `record cancel`

### Example

```toml
# Default post-processing
[output.post_process]
command = "ollama run llama3.2:1b 'Clean up:'"
timeout_ms = 30000

# Profile: casual chat
[profiles.slack]
post_process_command = "ollama run llama3.2:1b 'Rewrite casually for Slack:'"

# Profile: code comments, output to clipboard
[profiles.code]
post_process_command = "ollama run llama3.2:1b 'Format as code comment:'"
output_mode = "clipboard"

# Profile: meeting notes with longer timeout
[profiles.notes]
post_process_command = "ollama run llama3.2:1b 'Convert to bullet points:'"
post_process_timeout_ms = 60000
```

---

## [text]

Controls text post-processing after transcription.

### spoken_punctuation

**Type:** Boolean
**Default:** `false`
**Required:** No

When `true`, converts spoken punctuation words into their symbol equivalents. Useful for developers and technical writing.

**Supported conversions:**

| Spoken | Symbol |
|--------|--------|
| `period` | `.` |
| `comma` | `,` |
| `question mark` | `?` |
| `exclamation mark` / `exclamation point` | `!` |
| `colon` | `:` |
| `semicolon` | `;` |
| `open paren` / `open parenthesis` | `(` |
| `close paren` / `close parenthesis` | `)` |
| `open bracket` | `[` |
| `close bracket` | `]` |
| `open brace` | `{` |
| `close brace` | `}` |
| `dash` / `hyphen` | `-` |
| `underscore` | `_` |
| `at sign` / `at symbol` | `@` |
| `hash` / `hashtag` | `#` |
| `dollar sign` | `$` |
| `percent` / `percent sign` | `%` |
| `ampersand` | `&` |
| `asterisk` | `*` |
| `plus` / `plus sign` | `+` |
| `equals` / `equals sign` | `=` |
| `slash` / `forward slash` | `/` |
| `backslash` | `\` |
| `pipe` | `\|` |
| `tilde` | `~` |
| `backtick` | `` ` `` |
| `single quote` | `'` |
| `double quote` | `"` |
| `new line` | newline character |
| `new paragraph` | double newline |
| `tab` | tab character |

**Example:**
```toml
[text]
spoken_punctuation = true
```

With this enabled, saying "function open paren close paren" produces `function()`.

### replacements

**Type:** Table (key-value pairs)
**Default:** `{}`
**Required:** No

Custom word replacements applied after transcription. Matching is case-insensitive but preserves word boundaries. Useful for:
- Correcting frequently misheard words
- Expanding abbreviations
- Fixing brand names or technical terms

**Example:**
```toml
[text]
replacements = { "vox type" = "voxtype", "oh marky" = "Omarchy" }
```

If Whisper transcribes "vox type" (or "Vox Type"), it will be replaced with "voxtype".

**Multiple replacements:**
```toml
[text.replacements]
"vox type" = "voxtype"
"oh marky" = "Omarchy"
"oh marchy" = "Omarchy"
"omar g" = "Omarchy"
"omar key" = "Omarchy"
```

### smart_auto_submit

**Type:** Boolean
**Default:** `false`
**Required:** No

When `true`, Voxtype watches for the word "submit" at the end of each transcription. If detected, it strips the word from the output and presses Enter - as if `auto_submit` had fired, but triggered by voice rather than being permanently on. Trailing punctuation on "submit" (e.g., "submit." from spoken punctuation) is handled correctly.

**Example:**

```toml
[text]
smart_auto_submit = true
```

Saying "send a reply to Alice submit" types "send a reply to Alice" and presses Enter.

**Per-recording override:**

```bash
# Enable for just this recording (even if config has it off)
voxtype record start --smart-auto-submit
voxtype record toggle --smart-auto-submit

# Disable for just this recording (even if config has it on)
voxtype record start --no-smart-auto-submit
```

**Environment variable:**

```bash
VOXTYPE_SMART_AUTO_SUBMIT=true voxtype
```

**Note:** `smart_auto_submit` is conditional - it only fires when you say "submit". The existing `auto_submit` option always presses Enter after every transcription. Use `smart_auto_submit` when you want the choice per dictation, and `auto_submit` when you always want Enter pressed.

### filter_filler_words

**Type:** Boolean
**Default:** `true`
**Required:** No

When `true` (the default), strips common filler words ("uh", "um", "er", ...) from each transcription before output. Matching is case-insensitive and respects word boundaries, so words like "umbrella" or "summer" are not affected. Surrounding commas, semicolons, and double spaces are cleaned up so the result reads naturally. Set to `false` to disable.

**Example:**

```toml
[text]
filter_filler_words = true
```

With this enabled:

- "Well, um, I think" becomes "Well, I think"
- "uh hello world" becomes "hello world"
- "hello world, uh." becomes "hello world."

**CLI flag:**

```bash
voxtype --filter-fillers       # force on (overrides config)
voxtype --no-filter-fillers    # force off (overrides config)
```

**Environment variable:**

```bash
VOXTYPE_FILTER_FILLERS=true voxtype
```

The filter runs before `replacements` and the `[post_process]` LLM hook, so any custom replacements still apply on top of filtered text.

### filler_words

**Type:** Array of strings
**Default:** `["uh", "um", "er", "ah", "eh", "hmm", "hm", "mm", "mhm"]`
**Required:** No

Words removed by the filler-word filter. The default list is conservative and includes only single-syllable disfluencies. Override it to add your own (for example "like" or "you know"), or to disable specific entries by replacing the list.

**Example:**

```toml
[text]
filter_filler_words = true
filler_words = ["uh", "um", "er", "like", "you know"]
```

Multi-word entries like "you know" are matched as a single phrase. Adding aggressive entries (such as "like") may strip legitimate uses of the word; keep the list conservative or disable the filter for technical writing.

---

## [vad]

Voice Activity Detection configuration. When enabled, VAD filters silence-only recordings before transcription, preventing Whisper hallucinations when processing silence.

### enabled

**Type:** Boolean
**Default:** `false`
**Required:** No

Enable Voice Activity Detection. When enabled, recordings with no detected speech are rejected before transcription, saving processing time and preventing hallucinations on silent audio.

**Example:**
```toml
[vad]
enabled = true
```

**CLI override:**
```bash
voxtype --vad daemon
```

### backend

**Type:** String (`auto`, `energy`, `whisper`)
**Default:** `auto`
**Required:** No

VAD detection algorithm to use:

- `auto` - Automatically select based on transcription engine:
  - Whisper engine: uses Whisper VAD (more accurate, requires model)
  - Parakeet engine: uses Energy VAD (fast, no model needed)
- `energy` - Simple RMS energy-based detection. Fast and works with any engine, no model download required.
- `whisper` - Silero VAD via whisper-rs. More accurate speech detection but requires downloading the VAD model with `voxtype setup vad`.

**Example:**
```toml
[vad]
enabled = true
backend = "energy"  # Use fast energy-based detection
```

**CLI override:**
```bash
voxtype --vad --vad-backend whisper daemon
```

### threshold

**Type:** Float (0.0 - 1.0)
**Default:** `0.5`
**Required:** No

Speech detection sensitivity threshold. Lower values are more sensitive (detect quieter speech), higher values are more aggressive (require louder speech).

- `0.0` - Very sensitive, may detect background noise as speech
- `0.5` - Balanced, filters silence while allowing normal speech (default)
- `1.0` - Aggressive, requires loud clear speech

**Example:**
```toml
[vad]
enabled = true
threshold = 0.3  # More sensitive
```

**CLI override:**
```bash
voxtype --vad --vad-threshold 0.7 daemon
```

### min_speech_duration_ms

**Type:** Integer
**Default:** `100`
**Required:** No

Minimum amount of detected speech (in milliseconds) required for a recording to be transcribed. Recordings with less speech than this threshold are rejected.

**Example:**
```toml
[vad]
enabled = true
min_speech_duration_ms = 200  # Require at least 200ms of speech
```

---

## [meeting]

Meeting mode configuration. Meeting mode provides continuous transcription with chunked processing, speaker diarization, and export capabilities.

### enabled

**Type:** Boolean
**Default:** `false`
**Required:** No

Enable meeting mode. When enabled, the `voxtype meeting start/stop` commands become available.

### chunk_duration_secs

**Type:** Integer
**Default:** `30`
**Required:** No

Duration of audio chunks in seconds. The daemon processes audio in chunks of this size.

### storage_path

**Type:** String
**Default:** `"auto"` (`~/.local/share/voxtype/meetings/`)
**Required:** No

Directory for meeting transcript storage.

### retain_audio

**Type:** Boolean
**Default:** `false`
**Required:** No

Keep raw audio chunk files after transcription. Useful for debugging or re-transcribing with different settings.

### max_duration_mins

**Type:** Integer
**Default:** `180`
**Required:** No

Maximum meeting duration in minutes. Set to `0` for unlimited.

---

## [meeting.audio]

Audio capture settings specific to meeting mode.

### mic_device

**Type:** String
**Default:** `"default"`
**Required:** No

Microphone device for meeting recording. Falls back to the main `[audio] device` setting if not specified.

### loopback_device

**Type:** String (`"auto"`, `"disabled"`, or PulseAudio source name)
**Default:** `"auto"`
**Required:** No

System audio loopback capture for recording remote meeting participants. Uses `parec` (PulseAudio recording client) to capture audio from a monitor source, which works with both PulseAudio and PipeWire.

- `"auto"` - Detect a monitor source automatically via `pactl`. Prefers a source that is currently RUNNING (active audio output).
- `"disabled"` - Mic-only capture, no loopback.
- Explicit source name - Use a specific PulseAudio/PipeWire source (e.g., `"alsa_output.pci-0000_00_1f.3.analog-stereo.monitor"`).

To list available monitor sources:

```bash
pactl list short sources | grep monitor
```

### echo_cancel

**Type:** String (`"auto"`, `"disabled"`)
**Default:** `"auto"`
**Required:** No

Echo cancellation mode for removing speaker bleed-through from the microphone signal when loopback capture is active.

- `"auto"` - Use GTCRN neural speech enhancement on mic audio before transcription, followed by a phrase-level transcript dedup pass. The GTCRN model (~523 KB) is automatically downloaded on first `voxtype meeting start`.
- `"disabled"` - No enhancement. Use this if you have system-level echo cancellation configured (e.g., PipeWire's `echo-cancel` module) or if you don't use loopback capture.

### vad_threshold

**Type:** Float
**Default:** `0.01`
**Required:** No

RMS threshold for meeting chunk voice activity detection. Lower values are more permissive and can help quiet microphones; higher values skip more low-level noise before transcription. Set to `0.0` to disable this pre-transcription gate.

For quiet USB/XLR mics, try `0.001`.

**Example:**
```toml
[meeting.audio]
loopback_device = "auto"
echo_cancel = "auto"  # GTCRN enhancement + transcript dedup
vad_threshold = 0.001  # Optional: quiet mic tuning
```

---

## [meeting.diarization]

Speaker diarization settings for meeting mode.

### enabled

**Type:** Boolean
**Default:** `true`
**Required:** No

Enable speaker diarization to identify different speakers in meeting transcripts.

### backend

**Type:** String (`"simple"`, `"ml"`, `"remote"`)
**Default:** `"simple"`
**Required:** No

Diarization backend to use:

- `"simple"` - Uses audio source (mic vs loopback) to attribute speech as "You" or "Remote". No model download required.
- `"ml"` - ONNX-based speaker embeddings (ECAPA-TDNN) to identify individual remote speakers. The model is downloaded automatically on first use. **Experimental:** speaker clustering works best with longer speech segments; short segments may produce too many unique speaker IDs.
- `"remote"` - Remote diarization API.

### max_speakers

**Type:** Integer
**Default:** `10`
**Required:** No

Maximum number of speakers to detect.

---

## [meeting.summary]

AI summarization settings for meeting transcripts.

### backend

**Type:** String (`"local"`, `"remote"`, `"disabled"`)
**Default:** `"disabled"`
**Required:** No

- `"local"` - Use Ollama for local summarization.
- `"remote"` - Use a remote API.
- `"disabled"` - No summarization.

### ollama_url

**Type:** String
**Default:** `"http://localhost:11434"`
**Required:** No (only used when `backend = "local"`)

### ollama_model

**Type:** String
**Default:** `"llama3.2"`
**Required:** No (only used when `backend = "local"`)

### timeout_secs

**Type:** Integer
**Default:** `120`
**Required:** No

Timeout for summarization requests.

---

## [status]

Controls status display icons for Waybar and other tray integrations.

### icon_theme

**Type:** String
**Default:** `"emoji"`
**Required:** No

The icon theme to use for status display. Determines which icons appear in Waybar and other integrations.

**Built-in themes:**

***Font-based themes*** (require specific fonts installed):

| Theme | idle | recording | transcribing | stopped | Font Required |
|-------|------|-----------|--------------|---------|---------------|
| `emoji` | 🎙️ | 🎤 | ⏳ | (empty) | None (default) |
| `nerd-font` | U+F130 | U+F111 | U+F110 | U+F131 | [Nerd Font](https://www.nerdfonts.com/) |
| `material` | U+F036C | U+F040A | U+F04CE | U+F036D | [Material Design Icons](https://materialdesignicons.com/) |
| `phosphor` | U+E43A | U+E438 | U+E225 | U+E43B | [Phosphor Icons](https://phosphoricons.com/) |
| `codicons` | U+EB51 | U+EBFC | U+EB4C | U+EB52 | [Codicons](https://github.com/microsoft/vscode-codicons) |
| `omarchy` | U+EC12 | U+EC1C | U+EC1C | U+EC12 | Omarchy font |

***Universal themes*** (no special fonts required):

| Theme | idle | recording | transcribing | stopped | Description |
|-------|------|-----------|--------------|---------|-------------|
| `minimal` | ○ | ● | ◐ | × | Simple Unicode circles |
| `dots` | ◯ | ⬤ | ◔ | ◌ | Geometric shapes |
| `arrows` | ▶ | ● | ↻ | ■ | Media player style |
| `text` | [MIC] | [REC] | [...] | [OFF] | Plain text labels |

**Icon codepoint reference:**

| Theme | idle | recording | transcribing | stopped |
|-------|------|-----------|--------------|---------|
| `nerd-font` | microphone | circle | spinner | microphone-slash |
| `material` | mdi-microphone | mdi-record | mdi-sync | mdi-microphone-off |
| `phosphor` | ph-microphone | ph-record | ph-circle-notch | ph-microphone-slash |
| `codicons` | codicon-mic | codicon-record | codicon-sync | codicon-mute |

**Custom theme:** Specify a path to a TOML file containing custom icons.

**Example:**
```toml
[status]
icon_theme = "nerd-font"
```

**Custom theme file format** (`~/.config/voxtype/icons.toml`):
```toml
idle = "🎙️"
recording = "🔴"
transcribing = "⏳"
stopped = ""
```

### [status.icons]

Per-state icon overrides. These take precedence over the theme.

**Type:** Table
**Default:** Empty (use theme icons)
**Required:** No

Override specific icons without creating a full custom theme.

**Example:**
```toml
[status]
icon_theme = "emoji"

[status.icons]
recording = "🔴"  # Override just the recording icon
```

### Waybar Integration

Voxtype outputs an `alt` field in JSON that enables Waybar's `format-icons` feature. You can either:

1. **Use voxtype's icon themes** (simpler):
   ```toml
   [status]
   icon_theme = "nerd-font"
   ```

2. **Override in Waybar config** (more control):
   ```jsonc
   "custom/voxtype": {
       "exec": "voxtype status --follow --format json",
       "return-type": "json",
       "format": "{icon}",
       "format-icons": {
           "idle": "",
           "recording": "",
           "transcribing": "",
           "stopped": ""
       },
       "tooltip": true
   }
   ```

The `alt` field values match state names: `idle`, `recording`, `transcribing`, `stopped`.

See [User Manual - Waybar Integration](USER_MANUAL.md#with-waybar-status-indicator) for complete setup instructions.

---

## state_file

**Type:** String
**Default:** `"auto"`
**Required:** No

Path to a state file for external integrations like Waybar or Polybar. When configured, the daemon writes its current state to this file whenever state changes.

**Values written:**
- `idle` - Ready for input
- `recording` - Push-to-talk active, capturing audio
- `transcribing` - Processing audio through Whisper

**Special values:**
- `"auto"` - Uses `$XDG_RUNTIME_DIR/voxtype/state` (default, recommended)
- `"disabled"` - Turns off state file (also accepts `"none"`, `"off"`, `"false"`)

**Example:**
```toml
# Use automatic location (default)
state_file = "auto"

# Or specify explicit path
state_file = "/tmp/voxtype-state"

# Disable state file
state_file = "disabled"
```

**Usage with `voxtype status`:**

Once enabled, you can monitor the state:

```bash
# One-shot check
voxtype status

# JSON output for scripts
voxtype status --format json

# Continuous monitoring (for Waybar)
voxtype status --follow --format json
```

**Waybar module example:**

```json
"custom/voxtype": {
    "exec": "voxtype status --follow --format json",
    "return-type": "json",
    "format": "{}",
    "tooltip": true
}
```

See [User Manual - Waybar Integration](USER_MANUAL.md#with-waybar-status-indicator) for complete setup instructions.

---

## CLI Overrides

Most configuration options can be overridden via command line:

| Config Option | CLI Flag |
|--------------|----------|
| Config file | `-c`, `--config` |
| hotkey.key | `--hotkey` |
| whisper.model | `--model` |
| output.mode = "clipboard" | `--clipboard` |
| output.mode = "paste" | `--paste` |
| status.icon_theme | `--icon-theme` (status subcommand) |
| Verbosity | `-v`, `-vv`, `-q` |

**Example:**
```bash
voxtype --hotkey PAUSE --model small.en --clipboard
voxtype status --format json --icon-theme nerd-font
```

---

## Environment Variables

### RUST_LOG

Controls log verbosity via the tracing crate.

```bash
RUST_LOG=debug voxtype
RUST_LOG=voxtype=trace voxtype
```

### XDG_CONFIG_HOME

Overrides the config directory location (default: `~/.config`).

```bash
XDG_CONFIG_HOME=/custom/config voxtype
# Looks for: /custom/config/voxtype/config.toml
```

### XDG_DATA_HOME

Overrides the data directory location (default: `~/.local/share`).

```bash
XDG_DATA_HOME=/custom/data voxtype
# Models stored in: /custom/data/voxtype/models/
```

### VOXTYPE_* Configuration Overrides

Any config file setting can be overridden via environment variable. These are applied after the config file is loaded but before CLI flags, following the priority order: defaults < config file < env vars < CLI flags.

**Hotkey:**

| Variable | Type | Config equivalent |
|----------|------|-------------------|
| `VOXTYPE_HOTKEY` | string | `hotkey.key` |
| `VOXTYPE_HOTKEY_ENABLED` | bool | `hotkey.enabled` |
| `VOXTYPE_CANCEL_KEY` | string | `hotkey.cancel_key` |

**Whisper / Engine:**

| Variable | Type | Config equivalent |
|----------|------|-------------------|
| `VOXTYPE_MODEL` | string | `whisper.model` |
| `VOXTYPE_ENGINE` | string | `engine` |
| `VOXTYPE_LANGUAGE` | string | `whisper.language` |
| `VOXTYPE_TRANSLATE` | bool | `whisper.translate` |
| `VOXTYPE_THREADS` | integer | `whisper.threads` |
| `VOXTYPE_GPU_ISOLATION` | bool | `whisper.gpu_isolation` |
| `VOXTYPE_GPU_DEVICE` | integer | `whisper.gpu_device` |
| `VOXTYPE_ON_DEMAND_LOADING` | bool | `whisper.on_demand_loading` |
| `VOXTYPE_REMOTE_ENDPOINT` | string | `whisper.remote_endpoint` |
| `VOXTYPE_WHISPER_API_KEY` | string | `whisper.remote_api_key` |

**Audio:**

| Variable | Type | Config equivalent |
|----------|------|-------------------|
| `VOXTYPE_AUDIO_DEVICE` | string | `audio.device` |
| `VOXTYPE_MAX_DURATION_SECS` | integer | `audio.max_duration_secs` |
| `VOXTYPE_AUDIO_FEEDBACK` | bool | `audio.feedback.enabled` |

**Output:**

| Variable | Type | Config equivalent |
|----------|------|-------------------|
| `VOXTYPE_OUTPUT_MODE` | string | `output.mode` |
| `VOXTYPE_APPEND_TEXT` | string | `output.append_text` |
| `VOXTYPE_AUTO_SUBMIT` | bool | `output.auto_submit` |
| `VOXTYPE_SHIFT_ENTER_NEWLINES` | bool | `output.shift_enter_newlines` |
| `VOXTYPE_PRE_TYPE_DELAY` | integer | `output.pre_type_delay_ms` |
| `VOXTYPE_TYPE_DELAY` | integer | `output.type_delay_ms` |
| `VOXTYPE_FALLBACK_TO_CLIPBOARD` | bool | `output.fallback_to_clipboard` |
| `VOXTYPE_PASTE_KEYS` | string | `output.paste_keys` |
| `VOXTYPE_DOTOOL_XKB_LAYOUT` | string | `output.dotool_xkb_layout` |
| `VOXTYPE_DOTOOL_XKB_VARIANT` | string | `output.dotool_xkb_variant` |
| `VOXTYPE_EITYPE_XKB_LAYOUT` | string | `output.eitype_xkb_layout` |
| `VOXTYPE_EITYPE_XKB_VARIANT` | string | `output.eitype_xkb_variant` |
| `VOXTYPE_SPOKEN_PUNCTUATION` | bool | `text.spoken_punctuation` |
| `VOXTYPE_SMART_AUTO_SUBMIT` | bool | `text.smart_auto_submit` |
| `VOXTYPE_FILTER_FILLERS` | bool | `text.filter_filler_words` |

Boolean values: `true`, `1` to enable; `false`, `0` to disable.

```bash
# Example: enable auto-submit and Shift+Enter via environment
VOXTYPE_AUTO_SUBMIT=true VOXTYPE_SHIFT_ENTER_NEWLINES=true voxtype

# Example: override model and language
VOXTYPE_MODEL=large-v3-turbo VOXTYPE_LANGUAGE=auto voxtype
```

---

## Example Configurations

### Minimal Configuration

```toml
[whisper]
model = "base.en"
```

### High Accuracy

```toml
[whisper]
model = "medium.en"
threads = 8

[output.notification]
on_transcription = true
```

### Low Latency

```toml
[whisper]
model = "tiny.en"

[audio]
max_duration_secs = 30

[output]
type_delay_ms = 0
```

### Multilingual

```toml
[whisper]
model = "large-v3"
language = "auto"
translate = true  # Translate to English
```

### GPU with VRAM Optimization

```toml
[whisper]
model = "large-v3-turbo"
on_demand_loading = true  # Free VRAM when not transcribing

[audio.feedback]
enabled = true  # Helpful feedback since model loading adds brief delay
theme = "default"
```

### Laptop with Hybrid Graphics (GPU Isolation)

```toml
[whisper]
model = "large-v3-turbo"
gpu_isolation = true  # Release GPU memory between transcriptions

[audio.feedback]
enabled = true
theme = "default"
```

GPU isolation runs transcription in a subprocess that exits after each recording, allowing the discrete GPU to power down. The model loads while you speak, so perceived latency is nearly identical to standard mode.

### Custom Hotkey

```toml
[hotkey]
key = "F13"
modifiers = ["LEFTCTRL", "LEFTSHIFT"]

[output]
mode = "clipboard"

[output.notification]
on_transcription = true
```

### Developer / Programmer

```toml
[whisper]
model = "base.en"

[text]
# Say "period" to get ".", "open paren" to get "(", etc.
spoken_punctuation = true

# Fix common misheard technical terms
[text.replacements]
javascript = "JavaScript"
typescript = "TypeScript"
python = "Python"
```

### Server/Headless

```toml
[hotkey]
key = "F24"

[output]
mode = "clipboard"

[output.notification]
on_recording_start = false
on_recording_stop = false
on_transcription = false  # No desktop notifications
```

### Compositor Keybindings (Hyprland/Sway)

```toml
[hotkey]
enabled = false  # Disable built-in hotkey, use compositor keybindings

# Required for toggle mode
state_file = "auto"

[whisper]
model = "base.en"

[audio.feedback]
enabled = true  # Audio cues helpful when using external triggers
```

Then configure your compositor:

**Hyprland** (`~/.config/hypr/hyprland.conf`):
```hyprlang
bind = SUPER, V, exec, voxtype record start
bindr = SUPER, V, exec, voxtype record stop
```

**Sway** (`~/.config/sway/config`):
```
bindsym $mod+v exec voxtype record start
bindsym --release $mod+v exec voxtype record stop
```

### Remote Transcription (Self-Hosted)

Offload transcription to a GPU server on your local network:

```toml
[whisper]
backend = "remote"
language = "en"

# Your whisper.cpp server
remote_endpoint = "http://192.168.1.100:8080"
remote_timeout_secs = 30
```

On your GPU server, run whisper.cpp server:
```bash
./server -m models/ggml-large-v3-turbo.bin --host 0.0.0.0 --port 8080
```

### Remote Transcription (OpenAI Cloud)

Use OpenAI's hosted Whisper API (requires API key, has privacy implications):

```toml
[whisper]
backend = "remote"
language = "en"
remote_endpoint = "https://api.openai.com"
remote_model = "whisper-1"
remote_timeout_secs = 30
# API key set via: export VOXTYPE_WHISPER_API_KEY="sk-..."
```

> **Note**: Cloud-based transcription sends your audio to third-party servers. See [User Manual - Remote Whisper Servers](USER_MANUAL.md#remote-whisper-servers) for privacy considerations.

### Multi-Model Setup

Use a fast model for everyday dictation with a more accurate model available on-demand:

```toml
[hotkey]
key = "SCROLLLOCK"
model_modifier = "LEFTSHIFT"  # Hold Shift + hotkey for secondary model

[whisper]
model = "base.en"                    # Fast model, always ready
secondary_model = "large-v3-turbo"   # Accurate model on-demand
available_models = ["medium.en"]     # Additional models for CLI
max_loaded_models = 2                # Keep 2 models in memory
cold_model_timeout_secs = 300        # Evict unused models after 5 min

[audio.feedback]
enabled = true  # Helpful when switching models
```

**Usage:**
- Normal hotkey press: Uses `base.en` (fast)
- Hold Shift + hotkey: Uses `large-v3-turbo` (accurate)
- CLI override: `voxtype record start --model medium.en`

**Download models first:**
```bash
voxtype setup --download --model base.en
voxtype setup --download --model large-v3-turbo
voxtype setup --download --model medium.en
```

**Compatibility:** Multi-model works with all modes:
- `on_demand_loading = true`: Models load in background during recording
- `gpu_isolation = true`: Fresh subprocess per transcription with requested model
- `backend = "remote"`: Model name passed to remote server

---

## OSD Frontend

The on-screen display has multiple frontend implementations. Pick which one
the `voxtype-osd` wrapper launches via `[osd] frontend`.

```toml
[osd]
frontend = "gtk4"           # Default. Uses voxtype-osd-gtk4.
# frontend = "native"       # wgpu/egui-based (voxtype-osd-native).
# frontend = "quickshell"   # QML/Quickshell launcher (voxtype-osd-quickshell).
```

If you pick `"quickshell"`, install the QML tree first so the launcher can
find it:

```bash
voxtype setup quickshell
```

That command copies the QML files into `$XDG_DATA_HOME/voxtype/quickshell/`
(or `~/.local/share/voxtype/quickshell/`), symlinks the
`voxtype-audio-bridge` sidecar into `$XDG_BIN_HOME/voxtype-audio-bridge`
(or `~/.local/bin/voxtype-audio-bridge`) so the QML waveform can find
it on PATH, and prints compositor binding examples for the Wave 2 engine
picker and meeting controls panels. The AUR packages already install
the system-wide copy under `/usr/share/voxtype/quickshell/` and ship the
bridge at `/usr/lib/voxtype/voxtype-audio-bridge`; the per-user QML copy
is only required for source builds or for customization, and the bridge
symlink is what puts the sidecar on PATH where the QML expects it. Pass
`--skip-bridge` if your install already has the bridge on PATH. See the
[user manual](USER_MANUAL.md#voxtype-setup-quickshell) for details.

### Quickshell OSD customization

The Quickshell frontend can customize the whole OSD without editing VoxType's
packaged QML. Normal users configure declarative recipes; advanced users can
explicitly opt into trusted custom QML packages.

```toml
[osd]
frontend = "quickshell"
style = "default"      # Built-in style, package name, or package path
# palette = "omarchy" # omit for auto, or use omarchy, fallback, package, custom
layout = "compact"    # compact, wide, minimal, tile, orb, custom

# Explicit trusted package path. Custom QML is not sandboxed.
# plugin_path = "~/.config/voxtype/osd/my-style"

[osd.frame]
background = "background" # semantic role, literal color, or "none"
border = "state"          # state, semantic role, literal color, or "none"
glow = true               # voice-reactive soft glow around the frame
halo = true               # outline halo, used most visibly by orb recipes

[[osd.visual.layers]]
type = "pulse"
source = "rms"
color = "accent"
order = 0
opacity = 0.25
radius = 12

[[osd.visual.layers]]
type = "bars"
source = "peak"
color = "accent"
order = 10
gain = 1.2
mirror = true
```

When `palette` is omitted, a selected package manifest may choose the palette;
otherwise VoxType falls back to Omarchy colors. With `palette = "omarchy"`,
recipe colors such as `accent`, `background`,
`foreground`, `success`, `warning`, and `error` resolve from the active
Omarchy theme at `~/.config/omarchy/current/theme/colors.toml`. Literal colors
such as `"#ff6600"` are allowed when a recipe needs to override the theme.

Recipe layer `type` can be `shadow`, `background`, `waveform`, `bars`,
`pulse`, `ring`, `meter`, `icon`, or `label`. Layer `source` can be `peak`,
`rms`, `vad`, `state`, or `none`. Layer tunables you don't set use each
layer type's own defaults; explicit values, including `0.0`, are honored.
On `meter` layers, `color` sets the low-zone color while the mid/high
gradient stops keep the `warning`/`error` roles; on `shadow` layers,
`color` tints the backdrop (default black).

`layout` controls the outer OSD frame. `compact`, `wide`, and `minimal` are
strip layouts; `tile` is a square card; `orb` is a circular frame intended for
ring-focused recipes.

`[osd.frame]` controls the host frame around the recipe. Set
`background = "none"` or `border = "none"` for frameless recipes; the visual
layers continue to render normally.

Shareable style packages are directories containing `voxtype-osd.toml`, optional
assets under `assets/`, and optionally a QML entry file. Package QML is trusted
code and only loads when the package is selected through `style` or
`plugin_path`. A manifest only overrides the `[osd]` fields it explicitly
sets: a package that ships only `[colors]` keeps your configured `layout`,
`[osd.frame]`, and `[[osd.visual.layers]]` recipe, and an explicit `palette`
in your config always beats the manifest's.

If `style` names a package that isn't installed, `plugin_path` doesn't point
at a package directory, or the manifest's `qml_entry` file is missing, the
Quickshell launcher exits with an error explaining what to fix instead of
silently falling back to the default style. `style` and `plugin_path` paths
may start with `~`.

```toml
# ~/.config/voxtype/osd/bars-plus/voxtype-osd.toml
name = "bars-plus"
version = "1.0.0"
palette = "package"      # Optional; host config can override it
layout = "wide"
# qml_entry = "CustomOsd.qml"

[colors]
accent = "#8BD5CA"
background = "rgba(20, 22, 26, 0.82)"

[[visual.layers]]
type = "bars"
source = "peak"
color = "accent"
order = 10
```

---

## Deprecated Options

The following configuration options are deprecated but still supported for backwards compatibility. They will log a warning when used.

| Deprecated Option | Replacement | Notes |
|-------------------|-------------|-------|
| `wtype_delay_ms` | `pre_type_delay_ms` | Renamed for clarity (applies to all output drivers, not just wtype) |
| `--wtype-delay` CLI flag | `--pre-type-delay` | CLI equivalent of the above |
