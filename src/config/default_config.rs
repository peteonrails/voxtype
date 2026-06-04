//! Default configuration template.

/// Default configuration file content
pub const DEFAULT_CONFIG: &str = r#"# Voxtype Configuration
#
# Location: ~/.config/voxtype/config.toml
# All settings can be overridden via CLI flags

# State file for external integrations (Waybar, polybar, etc.)
# Use "auto" for default location ($XDG_RUNTIME_DIR/voxtype/state),
# a custom path, or "disabled" to turn off. The daemon writes state
# ("idle", "recording", "transcribing") to this file whenever it changes.
# Required for `voxtype record toggle` and `voxtype status` commands.
state_file = "auto"

[hotkey]
# Key to hold for push-to-talk
# Common choices: SCROLLLOCK, PAUSE, RIGHTALT, F13-F24
# Use `evtest` to find key names for your keyboard
key = "SCROLLLOCK"

# Optional modifier keys that must also be held
# Example: modifiers = ["LEFTCTRL", "LEFTALT"]
modifiers = []

# Activation mode: "push_to_talk" or "toggle"
# - push_to_talk: Hold hotkey to record, release to transcribe (default)
# - toggle: Press hotkey once to start recording, press again to stop
# mode = "push_to_talk"

# Enable built-in hotkey detection (default: true)
# Set to false when using compositor keybindings (Hyprland, Sway) instead
# When disabled, use `voxtype record start/stop/toggle` to control recording
# enabled = true

# Modifier key to select secondary model (evdev input mode only)
# When held while pressing the hotkey, uses whisper.secondary_model instead
# Example: model_modifier = "LEFTSHIFT"  # Shift+hotkey uses secondary model
# model_modifier = "LEFTSHIFT"

[audio]
# Audio input device ("default" uses system default)
# List devices with: pactl list sources short
device = "default"

# Sample rate in Hz (whisper expects 16000)
sample_rate = 16000

# Maximum recording duration in seconds (safety limit)
max_duration_secs = 60

# Pause MPRIS media players (Spotify, Firefox, etc.) when recording starts,
# resume them when recording stops. Requires playerctl.
# pause_media = false

# [audio.feedback]
# Enable audio feedback sounds (beeps when recording starts/stops)
# enabled = true
#
# Sound theme: "default", "subtle", "mechanical", or path to custom theme directory
# theme = "default"
#
# Volume level (0.0 to 1.0)
# volume = 0.7

[whisper]
# Transcription backend: "local" or "remote"
# - local: Use whisper.cpp locally (default)
# - remote: Send audio to a remote whisper.cpp server or OpenAI-compatible API
# backend = "local"

# Model to use for transcription (local backend)
# Options: tiny, tiny.en, base, base.en, small, small.en, medium, medium.en, large-v3, large-v3-turbo
# .en models are English-only but faster and more accurate for English
# large-v3-turbo is faster than large-v3 with minimal accuracy loss (recommended for GPU)
# Or provide absolute path to a custom .bin model file
model = "base.en"

# Language for transcription
# Options:
#   - Single language: "en", "fr", "de", etc.
#   - Auto-detect all: "auto"
#   - Constrained auto-detect: ["en", "fr"] (detects from allowed set only)
# The array form helps with multilingual users where Whisper might misdetect
# the language, especially for short sentences.
# See: https://github.com/openai/whisper#available-models-and-languages
language = "en"

# Translate non-English speech to English
translate = false

# Number of CPU threads for inference (omit for auto-detection)
# threads = 4

# GPU device index for Vulkan/CUDA backend selection.
# On multi-GPU systems, whisper.cpp may default to the integrated GPU (index 0),
# causing slower transcription. For different-vendor setups (e.g., Intel iGPU +
# NVIDIA dGPU), use VOXTYPE_VULKAN_DEVICE=nvidia instead (simpler).
# Use gpu_device for same-vendor GPUs or precise index control.
# gpu_device = 1

# Enable flash attention for GPU inference (default: false)
# Reduces memory usage (~75%) and improves speed (~10%) on CUDA/Vulkan.
# flash_attention = false

# Initial prompt to provide context for transcription
# Use this to hint at terminology, proper nouns, or formatting conventions.
# Example: "Technical discussion about Rust, TypeScript, and Kubernetes."
# initial_prompt = ""

# --- Multi-model settings ---
#
# Secondary model for difficult audio (used with hotkey.model_modifier or CLI --model)
# secondary_model = "large-v3-turbo"
#
# List of available models that can be requested via CLI --model flag
# available_models = ["large-v3-turbo", "medium.en"]
#
# Maximum models to keep loaded in memory (LRU eviction when exceeded)
# Default: 2 (primary + one secondary). Only applies when gpu_isolation = false.
# max_loaded_models = 2
#
# Seconds before unloading idle secondary models (0 = never auto-unload)
# Default: 300 (5 minutes). Only applies when gpu_isolation = false.
# cold_model_timeout_secs = 300

# --- Eager processing settings ---
#
# Enable eager input processing (transcribe chunks while recording continues)
# Reduces perceived latency on slower machines by processing audio in parallel.
# eager_processing = false
#
# Duration of each audio chunk in seconds (default: 5.0)
# eager_chunk_secs = 5.0
#
# Overlap between chunks in seconds (helps catch words at boundaries, default: 0.5)
# eager_overlap_secs = 0.5

# --- Remote backend settings (used when backend = "remote") ---
#
# Remote server endpoint URL (required for remote backend)
# Examples:
#   - whisper.cpp server: "http://192.168.1.100:8080"
#   - OpenAI API: "https://api.openai.com"
# remote_endpoint = "http://192.168.1.100:8080"
#
# Model name to send to remote server (default: "whisper-1")
# remote_model = "whisper-1"
#
# API key for remote server (optional, or use VOXTYPE_WHISPER_API_KEY env var)
# remote_api_key = ""
#
# Timeout for remote requests in seconds (default: 30)
# remote_timeout_secs = 30

[output]
# Primary output mode: "type" or "clipboard"
# - type: Simulates keyboard input at cursor position (requires ydotool)
# - clipboard: Copies text to clipboard (requires wl-copy)
mode = "type"

# Fall back to clipboard if typing fails
fallback_to_clipboard = true

# Custom driver order for type mode (optional)
# Default order: wtype -> dotool -> ydotool -> clipboard
# Customize to prefer a specific driver or change the fallback order.
# Available drivers: wtype, dotool, ydotool, clipboard
# Example: prefer ydotool over dotool:
#   driver_order = ["wtype", "ydotool", "dotool", "clipboard"]
# Example: use only ydotool, no fallback:
#   driver_order = ["ydotool"]
# driver_order = ["wtype", "dotool", "ydotool", "clipboard"]

# Per-language XKB layout variants for multilingual dictation.
# Use this with language arrays such as `language = ["en", "ru"]` when a
# language needs a variant that should not apply to other languages.
# Built-in layout defaults already include `ru = "ru"` in language_to_layout.
# [output.language_to_variant]
# ru = "phonetic"

# Delay between typed characters in milliseconds
# 0 = fastest possible, increase if characters are dropped
type_delay_ms = 0

# Automatically submit (send Enter key) after outputting transcribed text
# Useful for chat applications, command lines, or forms where you want
# to auto-submit after dictation
# auto_submit = true

# Convert newlines to Shift+Enter instead of regular Enter
# Useful for applications where Enter submits (e.g., Cursor IDE, Slack, Discord)
# shift_enter_newlines = false

# Prefix wtype output with a Shift key press/release
# Workaround for apps (e.g., Discord) that drop the first CJK character
# wtype_shift_prefix = false

# Restore clipboard content after paste mode (default: false)
# Saves clipboard before transcription, restores it after paste keystroke
# Only applies to mode = "paste". Useful when you want to preserve your
# existing clipboard content across dictation operations.
# restore_clipboard = false

# Delay after paste before restoring clipboard (milliseconds)
# Allows time for the paste operation to complete (default: 200)
# restore_clipboard_delay_ms = 200

# Wait for modifier keys to be released before typing (default: true)
# Prevents transcribed letters from combining with held modifiers (Ctrl/Alt/
# Shift/Super) and triggering compositor or application keybindings. Reads
# modifier state via evdev; requires user to be in the 'input' group.
# Silently disabled when /dev/input is unreadable, so output proceeds as
# before in that case.
# wait_for_modifier_release = true

# Maximum milliseconds to wait for modifier release before falling back to
# clipboard output. Prevents a stuck modifier from indefinitely blocking
# transcription delivery. (default: 750)
# modifier_release_timeout_ms = 750

# Pre/post output hooks (optional)
# Commands to run before and after typing output. Useful for compositor integration.
# Example: Block modifier keys during typing with Hyprland submap:
#   pre_output_command = "hyprctl dispatch submap voxtype_suppress"
#   post_output_command = "hyprctl dispatch submap reset"
# See troubleshooting docs for the required Hyprland submap configuration.
# Note: usually unnecessary now that wait_for_modifier_release is enabled by
# default; the submap workaround is only needed if /dev/input is unreadable.

# Post-processing command (optional)
# Pipe transcribed text through an external command for cleanup before output.
# The command receives text on stdin and outputs processed text on stdout.
# Useful for LLM-based text cleanup, grammar correction, filler word removal.
# On any failure (timeout, error), falls back to original transcription.
#
# [output.post_process]
# command = "ollama run llama3.2:1b 'Clean up this dictation. Fix grammar, remove filler words. Output only the cleaned text:'"
# timeout_ms = 30000  # 30 second timeout (generous for LLM)
# trim = true         # Strip leading/trailing whitespace from output (default: true)
# fallback_on_empty = true  # Use original text if command returns empty (default: true)

[output.notification]
# Show notification when recording starts (hotkey pressed)
on_recording_start = false

# Show notification when recording stops (transcription beginning)
on_recording_stop = false

# Show notification with transcribed text after transcription completes
on_transcription = true

# Notification urgency level: "low", "normal", or "critical".
# On GNOME, "low" notifications are delivered to the drawer without a popup banner.
# Use "normal" (default) to ensure notifications appear as banners.
# urgency = "normal"

# [text]
# Text processing options (word replacements, spoken punctuation)
#
# Enable spoken punctuation conversion (e.g., say "period" to get ".")
# spoken_punctuation = false
#
# Custom word replacements (case-insensitive)
# replacements = { "vox type" = "voxtype" }
#
# Smart auto-submit: say "submit" at the end of dictation to press Enter.
# The word "submit" is stripped from the output text and Enter is pressed.
# smart_auto_submit = false
#
# Remove filler words like "uh" and "um" from transcribed text.
# Enabled by default. Set filter_filler_words = false to disable, or override
# the word list via filler_words.
# filter_filler_words = true
# filler_words = ["uh", "um", "er", "ah", "eh", "hmm", "hm", "mm", "mhm"]

# [vad]
# Voice Activity Detection - filters silence-only recordings
# Prevents Whisper hallucinations on silent audio
#
# enabled = false      # Enable VAD (off by default)
# threshold = 0.5      # 0.0 = sensitive, 1.0 = aggressive
# min_speech_duration_ms = 100  # Minimum speech required

# [status]
# Status display icons for Waybar/tray integrations
#
# Icon theme (or path to custom theme file):
#   Font-based (require specific fonts):
#     - "emoji"     - Default emoji icons (🎙️ 🎤 ⏳)
#     - "nerd-font" - Nerd Font icons (requires Nerd Font)
#     - "material"  - Material Design Icons (requires MDI font)
#     - "phosphor"  - Phosphor Icons (requires Phosphor font)
#     - "codicons"  - VS Code icons (requires Codicons font)
#     - "omarchy"   - Omarchy distro icons
#   Universal (no special fonts needed):
#     - "minimal"   - Simple Unicode (○ ● ◐ ×)
#     - "dots"      - Geometric shapes (◯ ⬤ ◔ ◌)
#     - "arrows"    - Media player style (▶ ● ↻ ■)
#     - "text"      - Plain text ([MIC] [REC] [...] [OFF])
# icon_theme = "emoji"
#
# Per-state icon overrides (optional, takes precedence over theme)
# [status.icons]
# idle = "🎙️"
# recording = "🎤"
# transcribing = "⏳"
# stopped = ""

# [recording]
# Queue normal-batch recordings while a previous normal batch is transcribing or outputting.
# queue_enabled = false
# Maximum number of stopped recordings that can wait, transcribe, or output.
# A live recording is not counted while active, but starting one requires one
# available stopped slot so stopping can enqueue it.
# Set to 0 or 1 to disable queueing. Minimum enabled value is 2.
# queue_size = 5

# [profiles]
# Named profiles for context-specific post-processing
# Use with: voxtype record start --profile slack
#
# [profiles.slack]
# post_process_command = "ollama run llama3.2:1b 'Format for Slack...'"
#
# [profiles.code]
# post_process_command = "ollama run llama3.2:1b 'Format as code comment...'"
# output_mode = "clipboard"
"#;

/// Return the default config content with platform-appropriate hotkey
pub fn default_config_content() -> String {
    #[cfg(target_os = "macos")]
    {
        DEFAULT_CONFIG
            .replace("key = \"SCROLLLOCK\"", "key = \"FN\"")
            .replace(
                "# Common choices: SCROLLLOCK, PAUSE, RIGHTALT, F13-F24",
                "# Common choices: FN, RIGHTALT, F13-F24",
            )
            .replace(
                "# Use `evtest` to find key names for your keyboard",
                "# FN (Globe key) is recommended on macOS",
            )
    }
    #[cfg(not(target_os = "macos"))]
    {
        DEFAULT_CONFIG.to_string()
    }
}
