//! Machine-readable allowlist of user-settable config keys.
//!
//! This is the single source of truth shared by `voxtype config set/unset/get`,
//! `voxtype config schema --json`, and any external settings panel (the
//! Omarchy/Quickshell panel is the first consumer). Entries mirror the rows
//! the `voxtype configure` TUI renders, so the two surfaces can't disagree
//! about which keys exist or which values are legal. Where the schema is
//! deliberately ahead of the TUI, the entry says so at its definition.
//!
//! Three invariants hold, and each is pinned by a test at the bottom of this
//! file:
//!
//! 1. Every [`KeySpec`] has a matching arm in [`resolve`], so `value` in the
//!    JSON schema is always the effective runtime value rather than a guess.
//!    This is what catches keys that the TUI writes but the config loader
//!    never reads — voxtype's config has no `deny_unknown_fields`, so a
//!    write to a nonexistent path succeeds silently and only a read-back
//!    exposes it.
//! 2. Every [`KeySpec`] round-trips: a sample value written through
//!    [`apply`] parses back through `load_config` and resolves to what we
//!    wrote.
//! 3. Every `Enum` choice deserializes into the real config structs.

use std::path::Path;

use serde_json::{json, Map, Value as Json};

use super::{ActivationMode, Config, LanguageConfig};
use crate::tui::ConfigEditor;

/// Version of the `voxtype config schema --json` document shape. Bump when
/// the envelope changes in a way a consumer must notice.
pub const SCHEMA_VERSION: u32 = 1;

/// Sections, in the order the TUI sidebar shows them. Used to group the
/// human-readable `voxtype config schema` table.
pub const SECTIONS: &[&str] = &[
    "Engine",
    "Hotkey",
    "Audio",
    "Output",
    "Text",
    "VAD",
    "Meeting",
    "Notifications",
    "OSD",
    "Status",
    "Advanced",
];

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum KeyType {
    Bool,
    Int {
        min: i64,
        max: i64,
    },
    Float {
        min: f64,
        max: f64,
    },
    String,
    /// Closed set of values. `open` means the listed choices are the useful
    /// presets but any non-empty string is accepted — used where the config
    /// field is a free-form string that also has canonical values (an evdev
    /// key name, a sound theme that may be a directory path, an OSD style
    /// that may be a package path). A UI should render these as an editable
    /// combo box rather than a fixed picker.
    Enum {
        choices: &'static [&'static str],
        open: bool,
    },
    /// Any string; the legal set is discovered at runtime via
    /// `voxtype info <source> --json`. Never rejected at set time, because
    /// the daemon's view of models/devices can differ from ours.
    DynamicEnum {
        source: &'static str,
    },
    /// A dotted-tail map, e.g. `text.replacements.<from>`. These are not
    /// enumerated as individual keys; the schema exposes the whole map
    /// separately.
    MapString,
}

impl KeyType {
    /// The `type` discriminator in the JSON schema document.
    pub fn tag(self) -> &'static str {
        match self {
            KeyType::Bool => "bool",
            KeyType::Int { .. } => "int",
            KeyType::Float { .. } => "float",
            KeyType::String => "string",
            KeyType::Enum { .. } => "enum",
            KeyType::DynamicEnum { .. } => "dynamic_enum",
            KeyType::MapString => "map_string",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct KeySpec {
    /// Dotted TOML path, e.g. `audio.feedback.volume`.
    pub key: &'static str,
    /// Table portion for [`ConfigEditor`] (`""` for a root key).
    pub table: &'static str,
    pub field: &'static str,
    pub ty: KeyType,
    pub section: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    /// Set when the key only has an effect while the root `engine` equals
    /// this value.
    pub engine: Option<&'static str>,
    /// Cargo feature that must be compiled in for the key to do anything.
    pub requires_feature: Option<&'static str>,
    pub restart_required: bool,
}

const fn spec(
    key: &'static str,
    table: &'static str,
    field: &'static str,
    ty: KeyType,
    section: &'static str,
    label: &'static str,
    description: &'static str,
) -> KeySpec {
    KeySpec {
        key,
        table,
        field,
        ty,
        section,
        label,
        description,
        engine: None,
        requires_feature: None,
        restart_required: true,
    }
}

impl KeySpec {
    const fn for_engine(mut self, engine: &'static str) -> Self {
        self.engine = Some(engine);
        self
    }

    /// Mark the key as belonging to an optionally-compiled engine: it is both
    /// engine-scoped and feature-gated under the same name.
    const fn for_onnx_engine(mut self, engine: &'static str) -> Self {
        self.engine = Some(engine);
        self.requires_feature = Some(engine);
        self
    }

    const fn gated(mut self, feature: &'static str) -> Self {
        self.requires_feature = Some(feature);
        self
    }

    const fn live(mut self) -> Self {
        self.restart_required = false;
        self
    }

    /// Is the Cargo feature this key depends on compiled into this binary?
    pub fn compiled(&self) -> bool {
        match self.requires_feature {
            None => true,
            Some(f) => feature_compiled(f),
        }
    }
}

/// Was this binary built with `feature`?
///
/// Only features that gate config keys are listed. Pinned against
/// [`crate::config_set::engine_feature_compiled`] by a test so the two
/// `cfg!` blocks can't drift.
pub fn feature_compiled(feature: &str) -> bool {
    match feature {
        "parakeet" => cfg!(feature = "parakeet"),
        "moonshine" => cfg!(feature = "moonshine"),
        "sensevoice" => cfg!(feature = "sensevoice"),
        "paraformer" => cfg!(feature = "paraformer"),
        "dolphin" => cfg!(feature = "dolphin"),
        "omnilingual" => cfg!(feature = "omnilingual"),
        "cohere" => cfg!(feature = "cohere"),
        "openvino" => cfg!(feature = "openvino-whisper"),
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Choice lists. Each mirrors the corresponding `*_CHOICES` const in src/tui/.
// ---------------------------------------------------------------------------

/// Engines `voxtype config set engine` accepts. Mirrors `ENGINE_CHOICES` in
/// `src/tui/engine.rs` and [`crate::config_set::ENGINE_NAMES`]. Note this is
/// deliberately narrower than [`super::TranscriptionEngine`], which also has
/// a `Soniox` variant that neither the TUI picker nor `config set engine`
/// offers today.
const ENGINE_CHOICES: &[&str] = crate::config_set::ENGINE_NAMES;

const WHISPER_MODE_CHOICES: &[&str] = &["local", "remote", "cli"];
const WHISPER_LANG_CHOICES: &[&str] = &[
    "auto", "en", "fr", "de", "it", "es", "pt", "nl", "pl", "zh", "ja", "ko", "ru", "ar",
];
const SENSEVOICE_LANG_CHOICES: &[&str] = &["auto", "zh", "en", "ja", "ko", "yue"];
const COHERE_LANG_CHOICES: &[&str] = &[
    "ar", "de", "en", "es", "fr", "hi", "it", "ja", "ko", "nl", "pt", "ru", "tr", "zh",
];
const OPENVINO_DEVICE_CHOICES: &[&str] = &["NPU", "GPU", "CPU", "AUTO"];
const PARAKEET_MODEL_TYPE_CHOICES: &[&str] = &["tdt", "ctc"];

const HOTKEY_MODE_CHOICES: &[&str] = &["push_to_talk", "toggle"];
const HOTKEY_KEY_CHOICES: &[&str] = &[
    "HOME",
    "PAUSE",
    "SCROLLLOCK",
    "INSERT",
    "MENU",
    "F13",
    "F14",
    "F15",
    "F16",
    "RIGHTCTRL",
    "RIGHTALT",
    "RIGHTMETA",
    "CAPSLOCK",
];
const HOTKEY_CANCEL_CHOICES: &[&str] = &["ESC", "BACKSPACE", "F12", "DELETE", "END"];
const HOTKEY_MODIFIER_CHOICES: &[&str] =
    &["LEFTSHIFT", "RIGHTSHIFT", "LEFTCTRL", "LEFTALT", "LEFTMETA"];

const FEEDBACK_THEME_CHOICES: &[&str] = &["default", "subtle", "mechanical"];
const OUTPUT_MODE_CHOICES: &[&str] = &["type", "clipboard", "paste", "file"];
const VAD_BACKEND_CHOICES: &[&str] = &["auto", "energy", "whisper"];
const LOOPBACK_CHOICES: &[&str] = &["auto", "disabled"];
const ECHO_CANCEL_CHOICES: &[&str] = &["auto", "disabled"];

const OSD_FRONTEND_CHOICES: &[&str] = &["gtk4", "native", "quickshell"];
const OSD_STYLE_CHOICES: &[&str] = &["default"];
/// `auto` is absent on purpose: the config field is `Option`, and "auto"
/// means absent. Use `voxtype config unset osd.palette`.
const OSD_PALETTE_CHOICES: &[&str] = &["omarchy", "fallback", "package", "custom"];
const OSD_LAYOUT_CHOICES: &[&str] = &["compact", "wide", "minimal", "tile", "orb", "custom"];
const OSD_POSITION_CHOICES: &[&str] = &[
    "bottom-center",
    "top-center",
    "bottom-left",
    "bottom-right",
    "top-left",
    "top-right",
];

const ICON_THEME_CHOICES: &[&str] = &[
    "emoji",
    "nerd-font",
    "material",
    "phosphor",
    "codicons",
    "omarchy",
    "minimal",
    "dots",
    "arrows",
    "text",
];

const fn closed(choices: &'static [&'static str]) -> KeyType {
    KeyType::Enum {
        choices,
        open: false,
    }
}

const fn open(choices: &'static [&'static str]) -> KeyType {
    KeyType::Enum {
        choices,
        open: true,
    }
}

/// The dotted prefix of the one map-valued key family.
pub const REPLACEMENTS_TABLE: &str = "text.replacements";

// ---------------------------------------------------------------------------
// The allowlist.
// ---------------------------------------------------------------------------

pub const CONFIG_KEYS: &[KeySpec] = &[
    // -- Engine -------------------------------------------------------------
    spec(
        "engine",
        "",
        "engine",
        closed(ENGINE_CHOICES),
        "Engine",
        "Transcription engine",
        "Which ASR engine transcribes your audio. Non-whisper engines must be compiled into the binary.",
    ),
    // whisper
    spec(
        "whisper.model",
        "whisper",
        "model",
        KeyType::DynamicEnum { source: "models" },
        "Engine",
        "Model",
        "Whisper model name (tiny, base.en, large-v3-turbo, ...) or an absolute path to a .bin file.",
    )
    .for_engine("whisper"),
    // Paired with `hotkey.model_modifier`: the modifier picks this model for
    // one recording. Listed here rather than under Advanced because it is a
    // model picker like `whisper.model`, and without it the modifier key is
    // settable with no way to say what it switches to. The `voxtype configure`
    // TUI has no row for this one yet.
    spec(
        "whisper.secondary_model",
        "whisper",
        "secondary_model",
        KeyType::DynamicEnum { source: "models" },
        "Engine",
        "Secondary model",
        "Model used for recordings started while hotkey.model_modifier is held. Unset means the modifier has no effect.",
    )
    .for_engine("whisper"),
    spec(
        "whisper.mode",
        "whisper",
        "mode",
        closed(WHISPER_MODE_CHOICES),
        "Engine",
        "Mode",
        "How whisper runs: in-process (local), against an OpenAI-compatible API (remote), or via the whisper-cli subprocess (cli).",
    )
    .for_engine("whisper"),
    spec(
        "whisper.language",
        "whisper",
        "language",
        open(WHISPER_LANG_CHOICES),
        "Engine",
        "Language",
        "Spoken language code, or auto to detect. A comma-separated list constrains auto-detection to those languages.",
    )
    .for_engine("whisper"),
    spec(
        "whisper.translate",
        "whisper",
        "translate",
        KeyType::Bool,
        "Engine",
        "Translate to English",
        "Translate non-English speech into English instead of transcribing it verbatim.",
    )
    .for_engine("whisper"),
    spec(
        "whisper.threads",
        "whisper",
        "threads",
        KeyType::Int { min: 1, max: 256 },
        "Engine",
        "Threads",
        "CPU threads for inference. Unset lets whisper.cpp pick.",
    )
    .for_engine("whisper"),
    spec(
        "whisper.initial_prompt",
        "whisper",
        "initial_prompt",
        KeyType::String,
        "Engine",
        "Initial prompt",
        "Text prepended to whisper's context to bias vocabulary and spelling.",
    )
    .for_engine("whisper"),
    spec(
        "whisper.remote_endpoint",
        "whisper",
        "remote_endpoint",
        KeyType::String,
        "Engine",
        "Remote endpoint",
        "Base URL of an OpenAI-compatible transcription API. Only used when mode = remote.",
    )
    .for_engine("whisper"),
    spec(
        "whisper.remote_api_key",
        "whisper",
        "remote_api_key",
        KeyType::String,
        "Engine",
        "Remote API key",
        "Bearer token for the remote transcription endpoint.",
    )
    .for_engine("whisper"),
    spec(
        "whisper.remote_model",
        "whisper",
        "remote_model",
        KeyType::String,
        "Engine",
        "Remote model",
        "Model name to request from the remote endpoint (e.g. whisper-1).",
    )
    .for_engine("whisper"),
    // parakeet
    spec(
        "parakeet.model",
        "parakeet",
        "model",
        KeyType::DynamicEnum { source: "models" },
        "Engine",
        "Model",
        "Parakeet model directory name under the voxtype models dir.",
    )
    .for_onnx_engine("parakeet"),
    spec(
        "parakeet.model_type",
        "parakeet",
        "model_type",
        closed(PARAKEET_MODEL_TYPE_CHOICES),
        "Engine",
        "Decoder type",
        "Force the decoder family instead of auto-detecting it from the model directory.",
    )
    .for_onnx_engine("parakeet"),
    spec(
        "parakeet.on_demand_loading",
        "parakeet",
        "on_demand_loading",
        KeyType::Bool,
        "Engine",
        "Load on demand",
        "Load the model when recording starts and unload at idle, trading first-keystroke latency for memory.",
    )
    .for_onnx_engine("parakeet"),
    // moonshine
    spec(
        "moonshine.model",
        "moonshine",
        "model",
        KeyType::DynamicEnum { source: "models" },
        "Engine",
        "Model",
        "Moonshine model name (tiny, base).",
    )
    .for_onnx_engine("moonshine"),
    spec(
        "moonshine.quantized",
        "moonshine",
        "quantized",
        KeyType::Bool,
        "Engine",
        "Quantized weights",
        "Use the quantized ONNX weights: smaller and faster, marginally less accurate.",
    )
    .for_onnx_engine("moonshine"),
    spec(
        "moonshine.threads",
        "moonshine",
        "threads",
        KeyType::Int { min: 1, max: 256 },
        "Engine",
        "Threads",
        "ONNX Runtime intra-op threads. Unset lets voxtype pick.",
    )
    .for_onnx_engine("moonshine"),
    spec(
        "moonshine.on_demand_loading",
        "moonshine",
        "on_demand_loading",
        KeyType::Bool,
        "Engine",
        "Load on demand",
        "Load the model when recording starts and unload at idle.",
    )
    .for_onnx_engine("moonshine"),
    // sensevoice
    spec(
        "sensevoice.model",
        "sensevoice",
        "model",
        KeyType::DynamicEnum { source: "models" },
        "Engine",
        "Model",
        "SenseVoice model directory name.",
    )
    .for_onnx_engine("sensevoice"),
    spec(
        "sensevoice.language",
        "sensevoice",
        "language",
        closed(SENSEVOICE_LANG_CHOICES),
        "Engine",
        "Language",
        "SenseVoice language hint, or auto.",
    )
    .for_onnx_engine("sensevoice"),
    spec(
        "sensevoice.use_itn",
        "sensevoice",
        "use_itn",
        KeyType::Bool,
        "Engine",
        "Inverse text normalization",
        "Emit numerals and punctuation rather than spelled-out words.",
    )
    .for_onnx_engine("sensevoice"),
    spec(
        "sensevoice.threads",
        "sensevoice",
        "threads",
        KeyType::Int { min: 1, max: 256 },
        "Engine",
        "Threads",
        "ONNX Runtime intra-op threads. Unset lets voxtype pick.",
    )
    .for_onnx_engine("sensevoice"),
    spec(
        "sensevoice.on_demand_loading",
        "sensevoice",
        "on_demand_loading",
        KeyType::Bool,
        "Engine",
        "Load on demand",
        "Load the model when recording starts and unload at idle.",
    )
    .for_onnx_engine("sensevoice"),
    // paraformer
    spec(
        "paraformer.model",
        "paraformer",
        "model",
        KeyType::DynamicEnum { source: "models" },
        "Engine",
        "Model",
        "Paraformer model directory name.",
    )
    .for_onnx_engine("paraformer"),
    spec(
        "paraformer.threads",
        "paraformer",
        "threads",
        KeyType::Int { min: 1, max: 256 },
        "Engine",
        "Threads",
        "ONNX Runtime intra-op threads. Unset lets voxtype pick.",
    )
    .for_onnx_engine("paraformer"),
    spec(
        "paraformer.on_demand_loading",
        "paraformer",
        "on_demand_loading",
        KeyType::Bool,
        "Engine",
        "Load on demand",
        "Load the model when recording starts and unload at idle.",
    )
    .for_onnx_engine("paraformer"),
    // dolphin
    spec(
        "dolphin.model",
        "dolphin",
        "model",
        KeyType::DynamicEnum { source: "models" },
        "Engine",
        "Model",
        "Dolphin model directory name.",
    )
    .for_onnx_engine("dolphin"),
    spec(
        "dolphin.threads",
        "dolphin",
        "threads",
        KeyType::Int { min: 1, max: 256 },
        "Engine",
        "Threads",
        "ONNX Runtime intra-op threads. Unset lets voxtype pick.",
    )
    .for_onnx_engine("dolphin"),
    spec(
        "dolphin.on_demand_loading",
        "dolphin",
        "on_demand_loading",
        KeyType::Bool,
        "Engine",
        "Load on demand",
        "Load the model when recording starts and unload at idle.",
    )
    .for_onnx_engine("dolphin"),
    // omnilingual
    spec(
        "omnilingual.model",
        "omnilingual",
        "model",
        KeyType::DynamicEnum { source: "models" },
        "Engine",
        "Model",
        "Omnilingual model directory name.",
    )
    .for_onnx_engine("omnilingual"),
    spec(
        "omnilingual.threads",
        "omnilingual",
        "threads",
        KeyType::Int { min: 1, max: 256 },
        "Engine",
        "Threads",
        "ONNX Runtime intra-op threads. Unset lets voxtype pick.",
    )
    .for_onnx_engine("omnilingual"),
    spec(
        "omnilingual.on_demand_loading",
        "omnilingual",
        "on_demand_loading",
        KeyType::Bool,
        "Engine",
        "Load on demand",
        "Load the model when recording starts and unload at idle.",
    )
    .for_onnx_engine("omnilingual"),
    // cohere
    spec(
        "cohere.model",
        "cohere",
        "model",
        KeyType::DynamicEnum { source: "models" },
        "Engine",
        "Model",
        "Cohere Transcribe model variant (quantization level).",
    )
    .for_onnx_engine("cohere"),
    spec(
        "cohere.language",
        "cohere",
        "language",
        closed(COHERE_LANG_CHOICES),
        "Engine",
        "Language",
        "Cohere Transcribe language code. Cohere supports 14 languages.",
    )
    .for_onnx_engine("cohere"),
    spec(
        "cohere.threads",
        "cohere",
        "threads",
        KeyType::Int { min: 1, max: 256 },
        "Engine",
        "Threads",
        "ONNX Runtime intra-op threads. Unset lets voxtype pick.",
    )
    .for_onnx_engine("cohere"),
    spec(
        "cohere.on_demand_loading",
        "cohere",
        "on_demand_loading",
        KeyType::Bool,
        "Engine",
        "Load on demand",
        "Load the model when recording starts and unload at idle.",
    )
    .for_onnx_engine("cohere"),
    // openvino
    spec(
        "openvino.model",
        "openvino",
        "model",
        KeyType::DynamicEnum { source: "models" },
        "Engine",
        "Model",
        "OpenVINO Whisper model name or model directory.",
    )
    .for_onnx_engine("openvino"),
    spec(
        "openvino.device",
        "openvino",
        "device",
        closed(OPENVINO_DEVICE_CHOICES),
        "Engine",
        "Device",
        "OpenVINO inference device to try first.",
    )
    .for_onnx_engine("openvino"),
    spec(
        "openvino.quantized",
        "openvino",
        "quantized",
        KeyType::Bool,
        "Engine",
        "Quantized",
        "Prefer int8 quantized model variants.",
    )
    .for_onnx_engine("openvino"),
    spec(
        "openvino.threads",
        "openvino",
        "threads",
        KeyType::Int { min: 1, max: 256 },
        "Engine",
        "Threads",
        "CPU inference threads. Unset lets voxtype pick.",
    )
    .for_onnx_engine("openvino"),
    spec(
        "openvino.language",
        "openvino",
        "language",
        open(WHISPER_LANG_CHOICES),
        "Engine",
        "Language",
        "Whisper language code.",
    )
    .for_onnx_engine("openvino"),
    spec(
        "openvino.translate",
        "openvino",
        "translate",
        KeyType::Bool,
        "Engine",
        "Translate",
        "Translate non-English speech to English.",
    )
    .for_onnx_engine("openvino"),
    spec(
        "openvino.on_demand_loading",
        "openvino",
        "on_demand_loading",
        KeyType::Bool,
        "Engine",
        "Load on demand",
        "Load the model when recording starts and unload at idle.",
    )
    .for_onnx_engine("openvino"),
    spec(
        "openvino.openvino_dir",
        "openvino",
        "openvino_dir",
        KeyType::String,
        "Engine",
        "Runtime directory",
        "OpenVINO GenAI installation directory containing shared libraries.",
    )
    .for_onnx_engine("openvino"),
    spec(
        "openvino.streaming",
        "openvino",
        "streaming",
        KeyType::Bool,
        "Engine",
        "Streaming",
        "Enable live transcription through the shared sliding-window engine.",
    )
    .for_onnx_engine("openvino"),
    // -- Hotkey -------------------------------------------------------------
    spec(
        "hotkey.enabled",
        "hotkey",
        "enabled",
        KeyType::Bool,
        "Hotkey",
        "Built-in hotkey listener",
        "Watch the keyboard via evdev. Turn this off when your compositor calls `voxtype record` instead.",
    ),
    spec(
        "hotkey.key",
        "hotkey",
        "key",
        open(HOTKEY_KEY_CHOICES),
        "Hotkey",
        "Push-to-talk key",
        "evdev KEY_* name without the prefix, e.g. SCROLLLOCK or F13.",
    ),
    spec(
        "hotkey.mode",
        "hotkey",
        "mode",
        closed(HOTKEY_MODE_CHOICES),
        "Hotkey",
        "Activation mode",
        "push_to_talk records while the key is held; toggle starts and stops on separate taps.",
    ),
    spec(
        "hotkey.cancel_key",
        "hotkey",
        "cancel_key",
        open(HOTKEY_CANCEL_CHOICES),
        "Hotkey",
        "Cancel key",
        "Key that aborts an in-flight recording or transcription.",
    ),
    spec(
        "hotkey.model_modifier",
        "hotkey",
        "model_modifier",
        open(HOTKEY_MODIFIER_CHOICES),
        "Hotkey",
        "Secondary-model modifier",
        "Hold this alongside the hotkey to transcribe with whisper.secondary_model.",
    ),
    // -- Audio --------------------------------------------------------------
    spec(
        "audio.device",
        "audio",
        "device",
        KeyType::DynamicEnum { source: "devices" },
        "Audio",
        "Input device",
        "Capture device name, or default to follow the system default.",
    ),
    spec(
        "audio.max_duration_secs",
        "audio",
        "max_duration_secs",
        KeyType::Int { min: 5, max: 3600 },
        "Audio",
        "Max recording length",
        "Safety cap in seconds; recording stops on its own at this point.",
    ),
    spec(
        "audio.pause_media",
        "audio",
        "pause_media",
        KeyType::Bool,
        "Audio",
        "Pause media while recording",
        "Pause MPRIS players when recording starts and resume when it stops.",
    ),
    spec(
        "audio.duck_media",
        "audio",
        "duck_media",
        KeyType::Bool,
        "Audio",
        "Duck media while recording",
        "Lower other streams' volume instead of pausing them.",
    ),
    spec(
        "audio.duck_media_volume_percent",
        "audio",
        "duck_media_volume_percent",
        KeyType::Int { min: 0, max: 150 },
        "Audio",
        "Ducked volume",
        "Target volume percentage for other streams while ducking.",
    ),
    spec(
        "audio.duck_media_fade_ms",
        "audio",
        "duck_media_fade_ms",
        KeyType::Int { min: 0, max: 5000 },
        "Audio",
        "Duck fade",
        "Milliseconds to fade media volume down and back up. 0 is instant.",
    ),
    spec(
        "audio.feedback.enabled",
        "audio.feedback",
        "enabled",
        KeyType::Bool,
        "Audio",
        "Sound cues",
        "Play a start and stop sound around each recording.",
    ),
    spec(
        "audio.feedback.theme",
        "audio.feedback",
        "theme",
        open(FEEDBACK_THEME_CHOICES),
        "Audio",
        "Sound theme",
        "Built-in cue set, or a path to a directory of custom .wav files.",
    ),
    spec(
        "audio.feedback.volume",
        "audio.feedback",
        "volume",
        KeyType::Float { min: 0.0, max: 1.0 },
        "Audio",
        "Cue volume",
        "Playback volume for the sound cues, 0.0 to 1.0.",
    ),
    // -- Output -------------------------------------------------------------
    spec(
        "output.mode",
        "output",
        "mode",
        closed(OUTPUT_MODE_CHOICES),
        "Output",
        "Delivery mode",
        "type simulates keystrokes, clipboard copies, paste copies then presses the paste keys, file appends to output.file_path.",
    ),
    spec(
        "output.fallback_to_clipboard",
        "output",
        "fallback_to_clipboard",
        KeyType::Bool,
        "Output",
        "Fall back to clipboard",
        "Copy the text instead of dropping it when every typing driver fails.",
    ),
    spec(
        "output.auto_submit",
        "output",
        "auto_submit",
        KeyType::Bool,
        "Output",
        "Press Enter after typing",
        "Send Return once the transcription has been typed.",
    ),
    spec(
        "output.shift_enter_newlines",
        "output",
        "shift_enter_newlines",
        KeyType::Bool,
        "Output",
        "Shift+Enter for newlines",
        "Type embedded newlines as Shift+Enter so chat apps don't submit early.",
    ),
    spec(
        "output.pre_type_delay_ms",
        "output",
        "pre_type_delay_ms",
        KeyType::Int { min: 0, max: 5000 },
        "Output",
        "Delay before typing",
        "Pause in milliseconds before the first keystroke, giving virtual keyboards time to attach.",
    ),
    spec(
        "output.append_text",
        "output",
        "append_text",
        KeyType::String,
        "Output",
        "Append after text",
        "String added to the end of every transcription, e.g. a trailing space.",
    ),
    spec(
        "output.post_process.command",
        "output.post_process",
        "command",
        KeyType::String,
        "Output",
        "Post-process command",
        "Shell command that receives the transcription on stdin and returns the cleaned text on stdout.",
    ),
    // -- Text ---------------------------------------------------------------
    spec(
        "text.spoken_punctuation",
        "text",
        "spoken_punctuation",
        KeyType::Bool,
        "Text",
        "Spoken punctuation",
        "Convert spoken names like \"period\" and \"new line\" into the characters they describe.",
    ),
    spec(
        "text.smart_auto_submit",
        "text",
        "smart_auto_submit",
        KeyType::Bool,
        "Text",
        "Say \"submit\" to press Enter",
        "Strip a trailing \"submit\" from the transcription and press Return instead.",
    ),
    spec(
        "text.filter_filler_words",
        "text",
        "filter_filler_words",
        KeyType::Bool,
        "Text",
        "Drop filler words",
        "Remove \"uh\", \"um\" and the rest of text.filler_words from the output.",
    ),
    spec(
        "text.replacements.<from>",
        REPLACEMENTS_TABLE,
        "<from>",
        KeyType::MapString,
        "Text",
        "Word replacements",
        "Case-insensitive substitutions applied to the transcription, keyed by the spoken form.",
    ),
    // -- VAD ----------------------------------------------------------------
    spec(
        "vad.enabled",
        "vad",
        "enabled",
        KeyType::Bool,
        "VAD",
        "Voice activity detection",
        "Reject silence-only recordings before they reach the transcriber.",
    ),
    spec(
        "vad.backend",
        "vad",
        "backend",
        closed(VAD_BACKEND_CHOICES),
        "VAD",
        "Backend",
        "auto picks per engine; energy is RMS-based with no model; whisper uses the bundled Silero model.",
    ),
    spec(
        "vad.threshold",
        "vad",
        "threshold",
        KeyType::Float { min: 0.0, max: 1.0 },
        "VAD",
        "Speech threshold",
        "Confidence required to call a recording speech. Higher rejects more.",
    ),
    // -- Meeting ------------------------------------------------------------
    spec(
        "meeting.enabled",
        "meeting",
        "enabled",
        KeyType::Bool,
        "Meeting",
        "Meeting mode",
        "Allow `voxtype meeting` to record and transcribe long sessions.",
    ),
    spec(
        "meeting.audio.loopback_device",
        "meeting.audio",
        "loopback_device",
        open(LOOPBACK_CHOICES),
        "Meeting",
        "Loopback capture",
        "Device that captures remote participants. auto detects a monitor source; disabled records only your mic.",
    ),
    spec(
        "meeting.audio.echo_cancel",
        "meeting.audio",
        "echo_cancel",
        closed(ECHO_CANCEL_CHOICES),
        "Meeting",
        "Echo cancellation",
        "auto runs GTCRN enhancement plus transcript dedup on the mic track. Set disabled if PipeWire already cancels echo.",
    ),
    spec(
        "meeting.diarization.enabled",
        "meeting.diarization",
        "enabled",
        KeyType::Bool,
        "Meeting",
        "Speaker diarization",
        "Split the transcript by speaker.",
    ),
    // -- Notifications ------------------------------------------------------
    spec(
        "output.notification.on_recording_start",
        "output.notification",
        "on_recording_start",
        KeyType::Bool,
        "Notifications",
        "Notify on record start",
        "Post a desktop notification when recording begins.",
    ),
    spec(
        "output.notification.on_recording_stop",
        "output.notification",
        "on_recording_stop",
        KeyType::Bool,
        "Notifications",
        "Notify on record stop",
        "Post a desktop notification when recording ends and transcription starts.",
    ),
    spec(
        "output.notification.on_transcription",
        "output.notification",
        "on_transcription",
        KeyType::Bool,
        "Notifications",
        "Notify with the text",
        "Post the transcribed text as a notification when it completes.",
    ),
    spec(
        "output.notification.show_engine_icon",
        "output.notification",
        "show_engine_icon",
        KeyType::Bool,
        "Notifications",
        "Engine icon in title",
        "Prefix the notification title with the active engine's icon.",
    ),
    // -- OSD ----------------------------------------------------------------
    spec(
        "osd.enabled",
        "osd",
        "enabled",
        KeyType::Bool,
        "OSD",
        "On-screen display",
        "Spawn the floating waveform panel while recording.",
    ),
    spec(
        "osd.frontend",
        "osd",
        "frontend",
        closed(OSD_FRONTEND_CHOICES),
        "OSD",
        "Frontend",
        "Which OSD renderer the daemon launches.",
    ),
    spec(
        "osd.style",
        "osd",
        "style",
        open(OSD_STYLE_CHOICES),
        "OSD",
        "Style",
        "Quickshell style name, package name, or package path.",
    ),
    spec(
        "osd.palette",
        "osd",
        "palette",
        closed(OSD_PALETTE_CHOICES),
        "OSD",
        "Palette source",
        "Where Quickshell recipes take their colors from. Unset lets the selected package decide.",
    ),
    spec(
        "osd.layout",
        "osd",
        "layout",
        closed(OSD_LAYOUT_CHOICES),
        "OSD",
        "Layout preset",
        "Arrangement preset for the Quickshell OSD host.",
    ),
    spec(
        "osd.position",
        "osd",
        "position",
        closed(OSD_POSITION_CHOICES),
        "OSD",
        "Screen anchor",
        "Corner or edge of the focused output the panel anchors to.",
    ),
    spec(
        "osd.width_px",
        "osd",
        "width_px",
        KeyType::Int { min: 80, max: 4096 },
        "OSD",
        "Width",
        "Panel width in physical pixels.",
    ),
    spec(
        "osd.height_px",
        "osd",
        "height_px",
        KeyType::Int { min: 16, max: 2048 },
        "OSD",
        "Height",
        "Panel height in physical pixels.",
    ),
    spec(
        "osd.margin_px",
        "osd",
        "margin_px",
        KeyType::Int { min: 0, max: 512 },
        "OSD",
        "Edge margin",
        "Gap from the screen edge in physical pixels. Corner anchors use this for both axes.",
    ),
    spec(
        "osd.top_margin",
        "osd",
        "top_margin",
        KeyType::Float { min: 0.0, max: 1.0 },
        "OSD",
        "Vertical position",
        "Top edge as a fraction of monitor height, matching swayosd. Only used by the centered anchors.",
    ),
    spec(
        "osd.opacity",
        "osd",
        "opacity",
        KeyType::Float { min: 0.0, max: 1.0 },
        "OSD",
        "Opacity",
        "Background opacity of the panel.",
    ),
    spec(
        "osd.waveform_window_secs",
        "osd",
        "waveform_window_secs",
        KeyType::Float { min: 0.5, max: 30.0 },
        "OSD",
        "Waveform window",
        "How many seconds of audio the waveform shows at once.",
    ),
    spec(
        "osd.peak_decay_db_per_sec",
        "osd",
        "peak_decay_db_per_sec",
        KeyType::Float {
            min: 0.0,
            max: 100.0,
        },
        "OSD",
        "Peak decay",
        "Rate in dB per second at which held peaks fall back.",
    ),
    spec(
        "osd.waveform_gain",
        "osd",
        "waveform_gain",
        KeyType::Float { min: 0.1, max: 50.0 },
        "OSD",
        "Waveform gain",
        "Visual gain applied before drawing. Lower for hot mics, raise for quiet ones.",
    ),
    // -- Status -------------------------------------------------------------
    spec(
        "status.icon_theme",
        "status",
        "icon_theme",
        closed(ICON_THEME_CHOICES),
        "Status",
        "Icon theme",
        "Glyph set used by `voxtype status` for Waybar and tray integrations.",
    )
    .live(),
    spec(
        "status.icons.idle",
        "status.icons",
        "idle",
        KeyType::String,
        "Status",
        "Idle icon",
        "Override the theme's idle glyph.",
    )
    .live(),
    spec(
        "status.icons.recording",
        "status.icons",
        "recording",
        KeyType::String,
        "Status",
        "Recording icon",
        "Override the theme's recording glyph.",
    )
    .live(),
    spec(
        "status.icons.transcribing",
        "status.icons",
        "transcribing",
        KeyType::String,
        "Status",
        "Transcribing icon",
        "Override the theme's transcribing glyph.",
    )
    .live(),
    spec(
        "status.icons.stopped",
        "status.icons",
        "stopped",
        KeyType::String,
        "Status",
        "Stopped icon",
        "Override the theme's stopped glyph.",
    )
    .live(),
    // -- Advanced -----------------------------------------------------------
    spec(
        "whisper.gpu_isolation",
        "whisper",
        "gpu_isolation",
        KeyType::Bool,
        "Advanced",
        "GPU isolation",
        "Transcribe in a child process that exits afterwards, so GPU memory is released between recordings.",
    )
    .for_engine("whisper"),
    spec(
        "whisper.on_demand_loading",
        "whisper",
        "on_demand_loading",
        KeyType::Bool,
        "Advanced",
        "Load on demand",
        "Load the whisper model when recording starts and unload at idle.",
    )
    .for_engine("whisper"),
    spec(
        "whisper.flash_attention",
        "whisper",
        "flash_attention",
        KeyType::Bool,
        "Advanced",
        "Flash attention",
        "Enable whisper.cpp's flash-attention kernels. Faster on supported GPUs.",
    )
    .for_engine("whisper"),
    spec(
        "whisper.eager_processing",
        "whisper",
        "eager_processing",
        KeyType::Bool,
        "Advanced",
        "Eager processing",
        "Transcribe overlapping chunks while you are still speaking to cut perceived latency.",
    )
    .for_engine("whisper"),
    spec(
        "whisper.gpu_device",
        "whisper",
        "gpu_device",
        KeyType::Int { min: 0, max: 15 },
        "Advanced",
        "GPU device index",
        "Which GPU to run on when more than one is present. Unset uses the first.",
    )
    .for_engine("whisper"),
    spec(
        "parakeet.streaming",
        "parakeet",
        "streaming",
        KeyType::Bool,
        "Advanced",
        "Streaming transcription",
        "Emit text while you speak instead of after you release the key. Forces toggle activation.",
    )
    .for_engine("parakeet")
    .gated("parakeet"),
];

// ---------------------------------------------------------------------------
// Lookup
// ---------------------------------------------------------------------------

/// The result of resolving a user-supplied dotted key.
#[derive(Debug, Clone)]
pub enum Found {
    /// A scalar key from [`CONFIG_KEYS`].
    Key(&'static KeySpec),
    /// An entry in a map-valued family: the spec plus the concrete tail
    /// (e.g. `btw` for `text.replacements.btw`).
    MapEntry {
        spec: &'static KeySpec,
        entry: String,
    },
}

impl Found {
    pub fn spec(&self) -> &'static KeySpec {
        match self {
            Found::Key(s) => s,
            Found::MapEntry { spec, .. } => spec,
        }
    }

    /// The `(table, field)` pair to hand to [`ConfigEditor`].
    pub fn target(&self) -> (&str, &str) {
        match self {
            Found::Key(s) => (s.table, s.field),
            Found::MapEntry { spec, entry } => (spec.table, entry.as_str()),
        }
    }

    /// The dotted key as the user would type it. For map entries this is the
    /// concrete path (`text.replacements.btw`), not the spec's placeholder
    /// (`text.replacements.<from>`), so echoed output names something the
    /// user can paste back.
    pub fn dotted_key(&self) -> String {
        match self {
            Found::Key(s) => s.key.to_string(),
            Found::MapEntry { spec, entry } => format!("{}.{}", spec.table, entry),
        }
    }
}

/// Look up a dotted key in the allowlist.
///
/// Exact matches win. Failing that, a key under a [`KeyType::MapString`]
/// family's table (`text.replacements.btw`) resolves to that family with the
/// remaining segment as the entry name. Map entry names cannot themselves be
/// dotted, since the tail would be ambiguous with a nested table.
pub fn find_key(dotted: &str) -> Option<Found> {
    if let Some(s) = CONFIG_KEYS.iter().find(|s| s.key == dotted) {
        // The literal placeholder form ("text.replacements.<from>") is a
        // documentation artifact, not a settable key.
        if matches!(s.ty, KeyType::MapString) {
            return None;
        }
        return Some(Found::Key(s));
    }
    for s in CONFIG_KEYS.iter().filter(|s| s.ty == KeyType::MapString) {
        let prefix = format!("{}.", s.table);
        if let Some(entry) = dotted.strip_prefix(&prefix) {
            if !entry.is_empty() && !entry.contains('.') {
                return Some(Found::MapEntry {
                    spec: s,
                    entry: entry.to_string(),
                });
            }
        }
    }
    None
}

/// Scalar keys only — the ones that appear in the schema's `keys` array.
pub fn scalar_keys() -> impl Iterator<Item = &'static KeySpec> {
    CONFIG_KEYS.iter().filter(|s| s.ty != KeyType::MapString)
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// A value that passed [`validate_value`] and is ready for [`apply`].
#[derive(Debug, Clone, PartialEq)]
pub enum TypedValue {
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
}

impl TypedValue {
    pub fn to_json(&self) -> Json {
        match self {
            TypedValue::Bool(b) => json!(b),
            TypedValue::Int(n) => json!(n),
            TypedValue::Float(f) => json!(f),
            TypedValue::Str(s) => json!(s),
        }
    }

    /// How the value is echoed back in the `Set <key> = <value>` line.
    pub fn display(&self) -> String {
        match self {
            TypedValue::Bool(b) => b.to_string(),
            TypedValue::Int(n) => n.to_string(),
            TypedValue::Float(f) => f.to_string(),
            TypedValue::Str(s) => format!("\"{}\"", s),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ValueError {
    #[error("'{value}' is not a boolean. Use true or false.")]
    NotBool { value: String },
    #[error("'{value}' is not an integer.")]
    NotInt { value: String },
    #[error("'{value}' is not a number.")]
    NotFloat { value: String },
    #[error("{value} is out of range for {key}; expected {min} to {max}.")]
    OutOfRange {
        key: &'static str,
        value: String,
        min: String,
        max: String,
    },
    #[error("'{value}' is not a valid {key}. Valid values: {choices}")]
    NotAChoice {
        key: &'static str,
        value: String,
        choices: String,
    },
    #[error("{key} cannot be set to an empty string. Use `voxtype config unset {key}` instead.")]
    Empty { key: &'static str },
}

fn parse_bool(raw: &str) -> Option<bool> {
    match raw.to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" | "enabled" => Some(true),
        "false" | "0" | "no" | "off" | "disabled" => Some(false),
        _ => None,
    }
}

/// Type-check and range-check a raw CLI string against a key's spec.
pub fn validate_value(spec: &KeySpec, raw: &str) -> Result<TypedValue, ValueError> {
    match spec.ty {
        KeyType::Bool => parse_bool(raw)
            .map(TypedValue::Bool)
            .ok_or_else(|| ValueError::NotBool {
                value: raw.to_string(),
            }),
        KeyType::Int { min, max } => {
            let n: i64 = raw.trim().parse().map_err(|_| ValueError::NotInt {
                value: raw.to_string(),
            })?;
            if n < min || n > max {
                return Err(ValueError::OutOfRange {
                    key: spec.key,
                    value: n.to_string(),
                    min: min.to_string(),
                    max: max.to_string(),
                });
            }
            Ok(TypedValue::Int(n))
        }
        KeyType::Float { min, max } => {
            let f: f64 = raw.trim().parse().map_err(|_| ValueError::NotFloat {
                value: raw.to_string(),
            })?;
            if !f.is_finite() || f < min || f > max {
                return Err(ValueError::OutOfRange {
                    key: spec.key,
                    value: f.to_string(),
                    min: min.to_string(),
                    max: max.to_string(),
                });
            }
            Ok(TypedValue::Float(f))
        }
        KeyType::Enum { choices, open } => {
            if choices.contains(&raw) {
                return Ok(TypedValue::Str(raw.to_string()));
            }
            if open {
                if raw.is_empty() {
                    return Err(ValueError::Empty { key: spec.key });
                }
                return Ok(TypedValue::Str(raw.to_string()));
            }
            Err(ValueError::NotAChoice {
                key: spec.key,
                value: raw.to_string(),
                choices: choices.join(", "),
            })
        }
        KeyType::String | KeyType::DynamicEnum { .. } | KeyType::MapString => {
            if raw.is_empty() {
                return Err(ValueError::Empty { key: spec.key });
            }
            Ok(TypedValue::Str(raw.to_string()))
        }
    }
}

/// Every ONNX engine's config table is `Option` in [`Config`] and its `model`
/// field has no serde default, so a table that exists at all must carry
/// `model`. Writing any *other* key into a table that isn't there yet
/// produces a document the loader rejects — `ConfigEditor::save()` would then
/// roll the whole edit back and the user would see a bare "missing field
/// `model`" for a command that never mentioned models.
///
/// Seed `model` with the engine's default first, which is exactly what the
/// TUI does when it materializes a fresh engine table (see the
/// `*_section_existed` guards in `src/tui/engine.rs`).
fn ensure_required_siblings(editor: &mut ConfigEditor, spec: &KeySpec) {
    let Some(engine) = spec.engine else { return };
    // Whisper's table is not optional and its `model` has a serde default.
    if engine == "whisper" || spec.field == "model" {
        return;
    }
    if editor.get_string(spec.table, "model").is_none() {
        editor.set_string(
            spec.table,
            "model",
            crate::model_catalog::default_model(engine),
        );
    }
}

/// Write a validated value into the editor, using the setter that matches the
/// declared type.
///
/// Floats deliberately go through `set_float`, not a formatted `set_string` —
/// writing `volume = "0.70"` is exactly the bug in #451 that made the daemon
/// refuse to load the config it had just written.
pub fn apply(editor: &mut ConfigEditor, found: &Found, value: &TypedValue) {
    ensure_required_siblings(editor, found.spec());
    let (table, field) = found.target();
    match value {
        TypedValue::Bool(b) => editor.set_bool(table, field, *b),
        TypedValue::Int(n) => editor.set_int(table, field, *n),
        TypedValue::Float(f) => editor.set_float(table, field, *f),
        TypedValue::Str(s) => editor.set_string(table, field, s),
    }
}

// ---------------------------------------------------------------------------
// Reading resolved values back out of a loaded Config
// ---------------------------------------------------------------------------

fn opt_str(v: Option<&String>) -> Json {
    match v {
        Some(s) => json!(s),
        None => Json::Null,
    }
}

/// Emit an `f32` without the noise its widening to `f64` introduces:
/// `json!(0.6f32)` serializes as 0.6000000238418579. Round-tripping through
/// the shortest decimal that identifies the f32 gives back 0.6, which is what
/// the user typed and what the config file contains.
fn f32_json(v: f32) -> Json {
    match v.to_string().parse::<f64>() {
        Ok(f) => json!(f),
        Err(_) => json!(v),
    }
}

fn language_json(l: &LanguageConfig) -> Json {
    json!(l.as_vec().join(","))
}

/// The effective value of `key` in a fully-resolved [`Config`], as JSON.
///
/// Returns `None` only for keys that aren't in the allowlist. Every
/// [`KeySpec`] must have an arm here; `schema_has_a_resolver_for_every_key`
/// enforces it. Absent optional values resolve to `null`; absent per-engine
/// tables resolve to that engine's compiled-in defaults, since that is what
/// the daemon would use.
pub fn resolve(key: &str, cfg: &Config) -> Option<Json> {
    // Per-engine tables are Option in Config. Fall back to the engine's
    // Default so `value` reflects what the daemon would actually run with.
    let pk = || cfg.parakeet.clone().unwrap_or_default();
    let mn = || cfg.moonshine.clone().unwrap_or_default();
    let sv = || cfg.sensevoice.clone().unwrap_or_default();
    let pf = || cfg.paraformer.clone().unwrap_or_default();
    let dol = || cfg.dolphin.clone().unwrap_or_default();
    let om = || cfg.omnilingual.clone().unwrap_or_default();
    let co = || cfg.cohere.clone().unwrap_or_default();
    let ov = || cfg.openvino.clone().unwrap_or_default();

    let v = match key {
        "engine" => json!(cfg.engine.name()),

        "whisper.model" => json!(cfg.whisper.model),
        "whisper.secondary_model" => opt_str(cfg.whisper.secondary_model.as_ref()),
        "whisper.mode" => {
            let mode = cfg.whisper.mode.or(cfg.whisper.backend).unwrap_or_default();
            json!(serde_json::to_value(mode).ok()?)
        }
        "whisper.language" => language_json(&cfg.whisper.language),
        "whisper.translate" => json!(cfg.whisper.translate),
        "whisper.threads" => match cfg.whisper.threads {
            Some(n) => json!(n),
            None => Json::Null,
        },
        "whisper.initial_prompt" => opt_str(cfg.whisper.initial_prompt.as_ref()),
        "whisper.remote_endpoint" => opt_str(cfg.whisper.remote_endpoint.as_ref()),
        "whisper.remote_api_key" => opt_str(cfg.whisper.remote_api_key.as_ref()),
        "whisper.remote_model" => opt_str(cfg.whisper.remote_model.as_ref()),
        "whisper.gpu_isolation" => json!(cfg.whisper.gpu_isolation),
        "whisper.on_demand_loading" => json!(cfg.whisper.on_demand_loading),
        "whisper.flash_attention" => json!(cfg.whisper.flash_attention),
        "whisper.eager_processing" => json!(cfg.whisper.eager_processing),
        "whisper.gpu_device" => match cfg.whisper.gpu_device {
            Some(n) => json!(n),
            None => Json::Null,
        },

        "parakeet.model" => json!(pk().model),
        "parakeet.model_type" => match pk().model_type {
            Some(t) => serde_json::to_value(t).ok()?,
            None => Json::Null,
        },
        "parakeet.on_demand_loading" => json!(pk().on_demand_loading),
        "parakeet.streaming" => json!(pk().streaming),

        "moonshine.model" => json!(mn().model),
        "moonshine.quantized" => json!(mn().quantized),
        "moonshine.threads" => match mn().threads {
            Some(n) => json!(n),
            None => Json::Null,
        },
        "moonshine.on_demand_loading" => json!(mn().on_demand_loading),

        "sensevoice.model" => json!(sv().model),
        "sensevoice.language" => json!(sv().language),
        "sensevoice.use_itn" => json!(sv().use_itn),
        "sensevoice.threads" => match sv().threads {
            Some(n) => json!(n),
            None => Json::Null,
        },
        "sensevoice.on_demand_loading" => json!(sv().on_demand_loading),

        "paraformer.model" => json!(pf().model),
        "paraformer.threads" => match pf().threads {
            Some(n) => json!(n),
            None => Json::Null,
        },
        "paraformer.on_demand_loading" => json!(pf().on_demand_loading),

        "dolphin.model" => json!(dol().model),
        "dolphin.threads" => match dol().threads {
            Some(n) => json!(n),
            None => Json::Null,
        },
        "dolphin.on_demand_loading" => json!(dol().on_demand_loading),

        "omnilingual.model" => json!(om().model),
        "omnilingual.threads" => match om().threads {
            Some(n) => json!(n),
            None => Json::Null,
        },
        "omnilingual.on_demand_loading" => json!(om().on_demand_loading),

        "cohere.model" => json!(co().model),
        "cohere.language" => json!(co().language),
        "cohere.threads" => match co().threads {
            Some(n) => json!(n),
            None => Json::Null,
        },
        "cohere.on_demand_loading" => json!(co().on_demand_loading),

        "openvino.model" => json!(ov().model),
        "openvino.device" => json!(ov().device),
        "openvino.quantized" => json!(ov().quantized),
        "openvino.threads" => match ov().threads {
            Some(n) => json!(n),
            None => Json::Null,
        },
        "openvino.language" => json!(ov().language),
        "openvino.translate" => json!(ov().translate),
        "openvino.on_demand_loading" => json!(ov().on_demand_loading),
        "openvino.openvino_dir" => opt_str(ov().openvino_dir.as_ref()),
        "openvino.streaming" => json!(ov().streaming),

        "hotkey.enabled" => json!(cfg.hotkey.enabled),
        "hotkey.key" => json!(cfg.hotkey.key),
        "hotkey.mode" => json!(match cfg.hotkey.mode {
            ActivationMode::PushToTalk => "push_to_talk",
            ActivationMode::Toggle => "toggle",
        }),
        "hotkey.cancel_key" => opt_str(cfg.hotkey.cancel_key.as_ref()),
        "hotkey.model_modifier" => opt_str(cfg.hotkey.model_modifier.as_ref()),

        "audio.device" => json!(cfg.audio.device),
        "audio.max_duration_secs" => json!(cfg.audio.max_duration_secs),
        "audio.pause_media" => json!(cfg.audio.pause_media),
        "audio.duck_media" => json!(cfg.audio.duck_media),
        "audio.duck_media_volume_percent" => json!(cfg.audio.duck_media_volume_percent),
        "audio.duck_media_fade_ms" => json!(cfg.audio.duck_media_fade_ms),
        "audio.feedback.enabled" => json!(cfg.audio.feedback.enabled),
        "audio.feedback.theme" => json!(cfg.audio.feedback.theme),
        "audio.feedback.volume" => f32_json(cfg.audio.feedback.volume),

        "output.mode" => serde_json::to_value(cfg.output.mode.clone()).ok()?,
        "output.fallback_to_clipboard" => json!(cfg.output.fallback_to_clipboard),
        "output.auto_submit" => json!(cfg.output.auto_submit),
        "output.shift_enter_newlines" => json!(cfg.output.shift_enter_newlines),
        "output.pre_type_delay_ms" => json!(cfg.output.pre_type_delay_ms),
        "output.append_text" => opt_str(cfg.output.append_text.as_ref()),
        "output.post_process.command" => match &cfg.output.post_process {
            Some(p) => json!(p.command),
            None => Json::Null,
        },

        "text.spoken_punctuation" => json!(cfg.text.spoken_punctuation),
        "text.smart_auto_submit" => json!(cfg.text.smart_auto_submit),
        "text.filter_filler_words" => json!(cfg.text.filter_filler_words),
        "text.replacements.<from>" => {
            let map: Map<String, Json> = cfg
                .text
                .replacements
                .iter()
                .map(|(k, v)| (k.clone(), json!(v)))
                .collect();
            Json::Object(map)
        }

        "vad.enabled" => json!(cfg.vad.enabled),
        "vad.backend" => serde_json::to_value(cfg.vad.backend).ok()?,
        "vad.threshold" => f32_json(cfg.vad.threshold),

        "meeting.enabled" => json!(cfg.meeting.enabled),
        "meeting.audio.loopback_device" => json!(cfg.meeting.audio.loopback_device),
        "meeting.audio.echo_cancel" => json!(cfg.meeting.audio.echo_cancel),
        "meeting.diarization.enabled" => json!(cfg.meeting.diarization.enabled),

        "output.notification.on_recording_start" => {
            json!(cfg.output.notification.on_recording_start)
        }
        "output.notification.on_recording_stop" => {
            json!(cfg.output.notification.on_recording_stop)
        }
        "output.notification.on_transcription" => json!(cfg.output.notification.on_transcription),
        "output.notification.show_engine_icon" => json!(cfg.output.notification.show_engine_icon),

        "osd.enabled" => json!(cfg.osd.enabled),
        "osd.frontend" => serde_json::to_value(cfg.osd.frontend).ok()?,
        "osd.style" => json!(cfg.osd.style),
        "osd.palette" => match cfg.osd.palette {
            Some(p) => serde_json::to_value(p).ok()?,
            None => Json::Null,
        },
        "osd.layout" => serde_json::to_value(cfg.osd.layout).ok()?,
        "osd.position" => serde_json::to_value(cfg.osd.position).ok()?,
        "osd.width_px" => json!(cfg.osd.width_px),
        "osd.height_px" => json!(cfg.osd.height_px),
        "osd.margin_px" => json!(cfg.osd.margin_px),
        "osd.top_margin" => f32_json(cfg.osd.top_margin),
        "osd.opacity" => f32_json(cfg.osd.opacity),
        "osd.waveform_window_secs" => f32_json(cfg.osd.waveform_window_secs),
        "osd.peak_decay_db_per_sec" => f32_json(cfg.osd.peak_decay_db_per_sec),
        "osd.waveform_gain" => f32_json(cfg.osd.waveform_gain),

        "status.icon_theme" => json!(cfg.status.icon_theme),
        "status.icons.idle" => opt_str(cfg.status.icons.idle.as_ref()),
        "status.icons.recording" => opt_str(cfg.status.icons.recording.as_ref()),
        "status.icons.transcribing" => opt_str(cfg.status.icons.transcribing.as_ref()),
        "status.icons.stopped" => opt_str(cfg.status.icons.stopped.as_ref()),

        _ => return None,
    };
    Some(v)
}

// ---------------------------------------------------------------------------
// JSON document assembly
// ---------------------------------------------------------------------------

/// The literal value present in the config file for this key, or `Null` when
/// the key is absent. Typed according to the spec so a hand-edited
/// `volume = 1` still reports as a number rather than an integer surprise.
fn file_value(editor: &ConfigEditor, spec: &KeySpec) -> Json {
    let Some(v) = editor.get_toml_value(spec.table, spec.field) else {
        return Json::Null;
    };
    match spec.ty {
        KeyType::Bool => v.as_bool().map(|b| json!(b)).unwrap_or(Json::Null),
        KeyType::Int { .. } => v.as_integer().map(|n| json!(n)).unwrap_or(Json::Null),
        KeyType::Float { .. } => v
            .as_float()
            .or_else(|| v.as_integer().map(|n| n as f64))
            .map(|f| json!(f))
            .unwrap_or(Json::Null),
        _ => match v.as_str() {
            Some(s) => json!(s),
            // `whisper.language` accepts an array form; report it in the
            // same comma-joined shape `value` uses.
            None => v
                .as_array()
                .map(|a| {
                    let joined: Vec<&str> = a.iter().filter_map(|i| i.as_str()).collect();
                    json!(joined.join(","))
                })
                .unwrap_or(Json::Null),
        },
    }
}

/// One entry in the schema document's `keys` array.
fn key_json(spec: &KeySpec, cfg: &Config, editor: &ConfigEditor) -> Json {
    let mut o = Map::new();
    o.insert("key".into(), json!(spec.key));
    o.insert("type".into(), json!(spec.ty.tag()));
    match spec.ty {
        KeyType::Int { min, max } => {
            o.insert("min".into(), json!(min));
            o.insert("max".into(), json!(max));
        }
        KeyType::Float { min, max } => {
            o.insert("min".into(), json!(min));
            o.insert("max".into(), json!(max));
        }
        KeyType::Enum { choices, open } => {
            o.insert("choices".into(), json!(choices));
            if open {
                o.insert("open".into(), json!(true));
            }
        }
        KeyType::DynamicEnum { source } => {
            o.insert("source".into(), json!(source));
        }
        _ => {}
    }
    o.insert("section".into(), json!(spec.section));
    o.insert("label".into(), json!(spec.label));
    o.insert("description".into(), json!(spec.description));
    o.insert("value".into(), resolve(spec.key, cfg).unwrap_or(Json::Null));
    o.insert("file_value".into(), file_value(editor, spec));
    o.insert("engine".into(), json!(spec.engine));
    o.insert("compiled".into(), json!(spec.compiled()));
    o.insert("restart_required".into(), json!(spec.restart_required));
    Json::Object(o)
}

/// Build the `voxtype config schema --json` document.
pub fn schema_json(cfg: &Config, path: &Path, editor: &ConfigEditor) -> Json {
    let keys: Vec<Json> = scalar_keys().map(|s| key_json(s, cfg, editor)).collect();
    let replacements: Map<String, Json> = cfg
        .text
        .replacements
        .iter()
        .map(|(k, v)| (k.clone(), json!(v)))
        .collect();
    // `voxtype_version` is this CLI's build. `daemon_version` is what is
    // actually serving dictation, which is routinely a different thing: an
    // upgrade installed but not restarted, an ExecStart override pointing at
    // a private build, or a /usr/local install shadowing a packaged one. A
    // settings UI that shows only the former tells the user a fix is live
    // while the process without it is still running.
    let daemon = crate::daemon_status::running_version();
    json!({
        "schema_version": SCHEMA_VERSION,
        "voxtype_version": env!("CARGO_PKG_VERSION"),
        "daemon_version": daemon.version(),
        "daemon_version_label": daemon.describe(),
        "daemon_version_differs": daemon.differs_from_caller(),
        "config_path": path.display().to_string(),
        "engine": cfg.engine.name(),
        "keys": keys,
        "replacements": Json::Object(replacements),
    })
}

/// Build the `voxtype config get --json` document: every allowlisted key
/// mapped to its resolved value, plus the replacements map.
pub fn get_all_json(cfg: &Config) -> Json {
    let mut o = Map::new();
    for spec in scalar_keys() {
        o.insert(
            spec.key.to_string(),
            resolve(spec.key, cfg).unwrap_or(Json::Null),
        );
    }
    let replacements: Map<String, Json> = cfg
        .text
        .replacements
        .iter()
        .map(|(k, v)| (k.clone(), json!(v)))
        .collect();
    o.insert(REPLACEMENTS_TABLE.to_string(), Json::Object(replacements));
    Json::Object(o)
}

/// Resolved value of a single key, including map entries.
pub fn resolve_found(found: &Found, cfg: &Config) -> Json {
    match found {
        Found::Key(s) => resolve(s.key, cfg).unwrap_or(Json::Null),
        Found::MapEntry { entry, .. } => match cfg.text.replacements.get(entry) {
            Some(v) => json!(v),
            None => Json::Null,
        },
    }
}

/// Literal file value of a single key, including map entries.
pub fn file_value_found(found: &Found, editor: &ConfigEditor) -> Json {
    match found {
        Found::Key(s) => file_value(editor, s),
        Found::MapEntry { spec, entry } => editor
            .get_string(spec.table, entry)
            .map(|s| json!(s))
            .unwrap_or(Json::Null),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::load_config;
    use crate::config_set;
    use std::collections::HashSet;
    use std::fs;
    use std::path::PathBuf;

    fn temp_config() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, crate::config::default_config_content()).unwrap();
        (dir, path)
    }

    /// A valid value to write when exercising a key generically.
    fn sample(spec: &KeySpec) -> String {
        match spec.ty {
            KeyType::Bool => "true".to_string(),
            KeyType::Int { min, max } => {
                // Prefer a value that isn't the default so a no-op write
                // can't pass the round-trip by accident.
                let mid = min + (max - min) / 3;
                mid.to_string()
            }
            KeyType::Float { min, max } => {
                // 0.25 steps are exact in binary floating point, so the
                // f32 fields don't come back with a long mantissa.
                let v = if min <= 0.25 && max >= 0.25 {
                    0.25
                } else {
                    min
                };
                v.to_string()
            }
            KeyType::Enum { choices, .. } => choices[choices.len() - 1].to_string(),
            KeyType::DynamicEnum { .. } => "test-value".to_string(),
            KeyType::String => "test-value".to_string(),
            KeyType::MapString => "test-value".to_string(),
        }
    }

    #[test]
    fn keys_are_unique() {
        let mut seen = HashSet::new();
        for s in CONFIG_KEYS {
            assert!(
                seen.insert(s.key),
                "duplicate key in CONFIG_KEYS: {}",
                s.key
            );
        }
    }

    #[test]
    fn keys_agree_with_their_table_and_field() {
        for s in CONFIG_KEYS {
            let expected = if s.table.is_empty() {
                s.field.to_string()
            } else {
                format!("{}.{}", s.table, s.field)
            };
            assert_eq!(
                s.key, expected,
                "key '{}' does not match table '{}' + field '{}'",
                s.key, s.table, s.field
            );
        }
    }

    #[test]
    fn sections_are_known() {
        for s in CONFIG_KEYS {
            assert!(
                SECTIONS.contains(&s.section),
                "key '{}' has unknown section '{}'",
                s.key,
                s.section
            );
        }
    }

    /// Invariant 1: no key can be listed without a way to read it back.
    ///
    /// This is what catches keys the TUI writes but the loader ignores.
    /// voxtype's config structs don't use `deny_unknown_fields`, so writing
    /// a nonexistent path succeeds silently — only a read-back exposes it.
    #[test]
    fn schema_has_a_resolver_for_every_key() {
        let cfg = Config::default();
        for s in CONFIG_KEYS {
            assert!(
                resolve(s.key, &cfg).is_some(),
                "no resolve() arm for '{}'; either add one or drop the key",
                s.key
            );
        }
    }

    /// Invariant 2: every key round-trips through the real loader.
    ///
    /// Write a sample value with the generic setter, reload the file through
    /// `load_config`, and check the resolved value is what we wrote. A key
    /// pointing at a TOML path the config structs don't have fails here.
    #[test]
    fn every_key_round_trips_through_the_config_loader() {
        for s in scalar_keys() {
            let (_dir, path) = temp_config();
            let value = sample(s);
            let typed = validate_value(s, &value)
                .unwrap_or_else(|e| panic!("sample '{}' rejected for {}: {}", value, s.key, e));

            let mut ed = ConfigEditor::load_from_path(path.clone()).unwrap();
            apply(&mut ed, &Found::Key(s), &typed);
            ed.save()
                .unwrap_or_else(|e| panic!("save failed after setting {}: {}", s.key, e));

            let cfg = load_config(Some(&path))
                .unwrap_or_else(|e| panic!("reload failed after setting {}: {}", s.key, e));
            let got = resolve(s.key, &cfg).unwrap();
            assert_eq!(
                got,
                typed.to_json(),
                "{} did not round-trip: wrote {:?}, read back {:?}",
                s.key,
                typed.to_json(),
                got
            );
        }
    }

    /// Invariant 3: every closed enum choice deserializes.
    #[test]
    fn every_enum_choice_deserializes() {
        for s in scalar_keys() {
            let KeyType::Enum { choices, open } = s.ty else {
                continue;
            };
            if open {
                // Open enums accept anything; the listed values are hints.
                continue;
            }
            for choice in choices {
                let (_dir, path) = temp_config();
                let typed = validate_value(s, choice).unwrap();
                let mut ed = ConfigEditor::load_from_path(path.clone()).unwrap();
                apply(&mut ed, &Found::Key(s), &typed);
                ed.save().unwrap_or_else(|e| {
                    panic!("choice '{}' for {} failed validation: {}", choice, s.key, e)
                });
                let cfg = load_config(Some(&path)).unwrap();
                assert_eq!(
                    resolve(s.key, &cfg).unwrap(),
                    json!(choice),
                    "choice '{}' for {} did not survive a reload",
                    choice,
                    s.key
                );
            }
        }
    }

    #[test]
    fn unset_restores_the_default() {
        let (_dir, path) = temp_config();
        let mut ed = ConfigEditor::load_from_path(path.clone()).unwrap();
        ed.set_bool("text", "spoken_punctuation", true);
        ed.save().unwrap();
        assert_eq!(
            resolve(
                "text.spoken_punctuation",
                &load_config(Some(&path)).unwrap()
            )
            .unwrap(),
            json!(true)
        );

        let mut ed = ConfigEditor::load_from_path(path.clone()).unwrap();
        ed.unset("text", "spoken_punctuation");
        ed.save().unwrap();
        assert_eq!(
            resolve(
                "text.spoken_punctuation",
                &load_config(Some(&path)).unwrap()
            )
            .unwrap(),
            json!(false),
            "unset should fall back to the serde default"
        );
    }

    #[test]
    fn find_key_resolves_scalars_and_map_entries() {
        assert!(matches!(find_key("hotkey.mode"), Some(Found::Key(_))));
        assert!(find_key("hotkey.nope").is_none());
        assert!(find_key("").is_none());

        match find_key("text.replacements.btw") {
            Some(Found::MapEntry { entry, .. }) => assert_eq!(entry, "btw"),
            other => panic!("expected a map entry, got {:?}", other),
        }
        // The placeholder form is documentation, not a settable key.
        assert!(find_key("text.replacements.<from>").is_none());
        // A dotted tail would be ambiguous with a nested table.
        assert!(find_key("text.replacements.a.b").is_none());
        assert!(find_key("text.replacements.").is_none());
    }

    #[test]
    fn map_entries_round_trip() {
        let (_dir, path) = temp_config();
        let found = find_key("text.replacements.btw").unwrap();
        let typed = validate_value(found.spec(), "by the way").unwrap();

        let mut ed = ConfigEditor::load_from_path(path.clone()).unwrap();
        apply(&mut ed, &found, &typed);
        ed.save().unwrap();

        let cfg = load_config(Some(&path)).unwrap();
        assert_eq!(
            cfg.text.replacements.get("btw").map(String::as_str),
            Some("by the way")
        );
        assert_eq!(resolve_found(&found, &cfg), json!("by the way"));

        let (table, field) = found.target();
        let mut ed = ConfigEditor::load_from_path(path.clone()).unwrap();
        ed.unset(table, field);
        ed.save().unwrap();
        let cfg = load_config(Some(&path)).unwrap();
        assert!(!cfg.text.replacements.contains_key("btw"));
    }

    #[test]
    fn float_keys_are_written_as_toml_numbers() {
        // Regression for #451: a formatted string here makes the daemon
        // refuse to load the config it just wrote.
        let (_dir, path) = temp_config();
        let spec = scalar_keys()
            .find(|s| s.key == "audio.feedback.volume")
            .unwrap();
        let typed = validate_value(spec, "0.25").unwrap();
        let mut ed = ConfigEditor::load_from_path(path.clone()).unwrap();
        apply(&mut ed, &Found::Key(spec), &typed);
        ed.save().unwrap();

        let text = fs::read_to_string(&path).unwrap();
        assert!(
            text.contains("volume = 0.25"),
            "expected a bare TOML float, got: {}",
            text
        );
        assert!(!text.contains("volume = \"0.25\""));
    }

    #[test]
    fn validate_rejects_out_of_range_and_unknown_choices() {
        let vol = scalar_keys()
            .find(|s| s.key == "audio.feedback.volume")
            .unwrap();
        assert!(matches!(
            validate_value(vol, "2.0"),
            Err(ValueError::OutOfRange { .. })
        ));
        assert!(matches!(
            validate_value(vol, "loud"),
            Err(ValueError::NotFloat { .. })
        ));

        let mode = scalar_keys().find(|s| s.key == "output.mode").unwrap();
        assert!(matches!(
            validate_value(mode, "telepathy"),
            Err(ValueError::NotAChoice { .. })
        ));

        let dur = scalar_keys()
            .find(|s| s.key == "audio.max_duration_secs")
            .unwrap();
        assert!(matches!(
            validate_value(dur, "0"),
            Err(ValueError::OutOfRange { .. })
        ));
        assert!(matches!(
            validate_value(dur, "1.5"),
            Err(ValueError::NotInt { .. })
        ));
    }

    #[test]
    fn open_enums_accept_values_outside_their_choice_list() {
        let key = scalar_keys().find(|s| s.key == "hotkey.key").unwrap();
        // F24 is a legitimate evdev key that the TUI's cycle list omits.
        assert_eq!(
            validate_value(key, "F24").unwrap(),
            TypedValue::Str("F24".into())
        );
        assert!(matches!(
            validate_value(key, ""),
            Err(ValueError::Empty { .. })
        ));
    }

    #[test]
    fn bool_accepts_common_spellings() {
        let s = scalar_keys().find(|s| s.key == "hotkey.enabled").unwrap();
        for raw in ["true", "TRUE", "1", "yes", "on"] {
            assert_eq!(validate_value(s, raw).unwrap(), TypedValue::Bool(true));
        }
        for raw in ["false", "0", "no", "off"] {
            assert_eq!(validate_value(s, raw).unwrap(), TypedValue::Bool(false));
        }
        assert!(validate_value(s, "maybe").is_err());
    }

    /// Pin the two `cfg!` blocks that answer "is this engine compiled in?".
    #[test]
    fn feature_gate_agrees_with_config_set() {
        for name in config_set::ENGINE_NAMES {
            if *name == "whisper" {
                continue; // always available, so it has no feature entry
            }
            assert_eq!(
                feature_compiled(name),
                config_set::engine_feature_compiled(name),
                "feature gate for '{}' disagrees with config_set",
                name
            );
        }
    }

    /// Every ONNX engine's keys must be gated, or a user on a whisper-only
    /// build gets told a key works when it does nothing.
    #[test]
    fn onnx_engine_keys_are_feature_gated() {
        for s in CONFIG_KEYS {
            let Some(engine) = s.engine else { continue };
            if engine == "whisper" {
                assert!(
                    s.requires_feature.is_none(),
                    "{} should not be feature-gated; whisper is always compiled in",
                    s.key
                );
            } else {
                assert_eq!(
                    s.requires_feature,
                    Some(engine),
                    "{} is an {} key and must be gated on the '{}' feature",
                    s.key,
                    engine,
                    engine
                );
            }
        }
    }

    /// `hotkey.model_modifier` only does something if the user can also name
    /// the model it switches to, so the two keys have to ship together.
    #[test]
    fn the_secondary_model_modifier_has_a_model_to_select() {
        let modifier = scalar_keys()
            .find(|s| s.key == "hotkey.model_modifier")
            .expect("hotkey.model_modifier is missing");
        assert!(modifier.description.contains("whisper.secondary_model"));

        let model = scalar_keys()
            .find(|s| s.key == "whisper.secondary_model")
            .expect(
                "hotkey.model_modifier points at whisper.secondary_model, which has no KeySpec",
            );
        assert_eq!(model.engine, Some("whisper"));
        assert!(
            matches!(model.ty, KeyType::DynamicEnum { source: "models" }),
            "secondary model should be picked from the same list as whisper.model, got {:?}",
            model.ty
        );
    }

    #[test]
    fn engine_choices_match_config_set() {
        assert_eq!(ENGINE_CHOICES, config_set::ENGINE_NAMES);
        let spec = scalar_keys().find(|s| s.key == "engine").unwrap();
        for name in ENGINE_CHOICES {
            assert!(validate_value(spec, name).is_ok(), "engine '{}'", name);
        }
        assert!(validate_value(spec, "nope").is_err());
    }

    #[test]
    fn schema_json_has_the_documented_envelope() {
        let (_dir, path) = temp_config();
        let cfg = load_config(Some(&path)).unwrap();
        let ed = ConfigEditor::load_from_path(path.clone()).unwrap();
        let doc = schema_json(&cfg, &path, &ed);

        assert_eq!(doc["schema_version"], json!(SCHEMA_VERSION));
        assert_eq!(doc["voxtype_version"], json!(env!("CARGO_PKG_VERSION")));
        assert_eq!(doc["config_path"], json!(path.display().to_string()));
        assert_eq!(doc["engine"], json!("whisper"));
        assert!(doc["replacements"].is_object());

        let keys = doc["keys"].as_array().unwrap();
        assert_eq!(keys.len(), scalar_keys().count());
        for k in keys {
            for field in [
                "key",
                "type",
                "section",
                "label",
                "description",
                "value",
                "file_value",
                "engine",
                "compiled",
                "restart_required",
            ] {
                assert!(
                    k.get(field).is_some(),
                    "key entry missing '{}': {}",
                    field,
                    k
                );
            }
            let ty = k["type"].as_str().unwrap();
            assert!(
                ["bool", "int", "float", "string", "enum", "dynamic_enum"].contains(&ty),
                "unexpected type tag '{}' in the schema document",
                ty
            );
            match ty {
                "int" | "float" => {
                    assert!(k.get("min").is_some() && k.get("max").is_some(), "{}", k)
                }
                "enum" => assert!(k["choices"].is_array(), "{}", k),
                "dynamic_enum" => assert!(k["source"].is_string(), "{}", k),
                _ => {}
            }
        }
        // MapString families are exposed via `replacements`, not `keys`.
        assert!(!keys
            .iter()
            .any(|k| k["key"] == json!("text.replacements.<from>")));
    }

    #[test]
    fn schema_json_reports_file_values_and_nulls() {
        let (_dir, path) = temp_config();
        let mut ed = ConfigEditor::load_from_path(path.clone()).unwrap();
        ed.set_string("hotkey", "mode", "toggle");
        ed.save().unwrap();

        let cfg = load_config(Some(&path)).unwrap();
        let ed = ConfigEditor::load_from_path(path.clone()).unwrap();
        let doc = schema_json(&cfg, &path, &ed);
        let keys = doc["keys"].as_array().unwrap();

        let mode = keys
            .iter()
            .find(|k| k["key"] == json!("hotkey.mode"))
            .unwrap();
        assert_eq!(mode["value"], json!("toggle"));
        assert_eq!(mode["file_value"], json!("toggle"));

        // An optional field nobody set reports null on both sides.
        let prompt = keys
            .iter()
            .find(|k| k["key"] == json!("whisper.initial_prompt"))
            .unwrap();
        assert_eq!(prompt["file_value"], Json::Null);
    }

    #[test]
    fn get_all_json_covers_every_scalar_key() {
        let cfg = Config::default();
        let all = get_all_json(&cfg);
        let o = all.as_object().unwrap();
        for s in scalar_keys() {
            assert!(
                o.contains_key(s.key),
                "missing {} in config get --json",
                s.key
            );
        }
        assert!(o.contains_key(REPLACEMENTS_TABLE));
    }

    #[test]
    fn uncompiled_engine_keys_report_compiled_false() {
        // On a default build the ONNX engines are absent, so their keys must
        // advertise that rather than pretending to work.
        for s in scalar_keys() {
            let Some(feature) = s.requires_feature else {
                continue;
            };
            assert_eq!(s.compiled(), feature_compiled(feature), "{}", s.key);
        }
    }
}
