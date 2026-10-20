//! Engine section: per-engine tunables for the active transcription engine.
//!
//! The first row picks the engine itself; subsequent rows show the fields
//! that engine actually has. Switching engines on the form does not change
//! the value of fields you've edited for other engines — they're held in
//! memory until you save.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    Frame,
};

use super::app::{Action, App};
use super::common::{
    self, FeedbackLevel as CommonFeedback, FormRowSpec, TextInput, TextInputResult,
};
use super::config_editor::{ConfigEditor, EditorError};
use super::theme;
use crate::config::TranscriptionEngine;
use crate::model_catalog::{default_model, installed_models_for, model_catalog};
use crate::setup::binary::{self, EngineFamily, InstallKind, Variant};
use crate::setup::model;
use strum::IntoEnumIterator;

#[derive(Debug, Clone)]
pub struct EngineState {
    pub engine: String,
    pub fields: AllFields,
    pub cursor: usize,
    pub feedback: Option<Feedback>,
    pub dirty_since_load: bool,
    /// If the chosen engine needs a different binary family than what's
    /// currently active, this holds the variant we'll switch to on save.
    /// `None` means no switch needed.
    pub pending_variant_switch: Option<Variant>,
    /// True when we wanted to switch but couldn't (source build, no
    /// installed variant supports the new engine, …). Surfaced as a warning
    /// on the screen.
    pub binary_switch_blocked: Option<String>,
    /// Active inline text edit. While `Some`, all keypresses route to the
    /// TextInput; navigation and cycle are suspended.
    pub editing: Option<TextEdit>,
}

#[derive(Debug, Clone)]
pub struct TextEdit {
    pub field: FieldId,
    pub input: TextInput,
}

#[derive(Debug, Clone, Default)]
pub struct AllFields {
    // Whisper
    pub w_model: String,
    pub w_mode: String,
    pub w_language: String,
    pub w_translate: bool,
    pub w_threads: Option<i64>,
    pub w_initial_prompt: Option<String>,
    pub w_flash_attention: bool,
    pub w_on_demand_loading: bool,
    pub w_gpu_isolation: bool,
    /// Only relevant when mode = "remote".
    pub w_remote_endpoint: Option<String>,
    pub w_remote_api_key: Option<String>,
    pub w_remote_model: Option<String>,

    // Parakeet
    pub pk_model: String,
    pub pk_model_type: Option<String>, // "tdt", "ctc", or None for auto-detect
    pub pk_on_demand_loading: bool,
    /// True if the [parakeet] table existed in the config at load time. We
    /// only write back to it on save if either this is true or parakeet is
    /// the active engine — otherwise saves leak partial tables that fail to
    /// deserialize because `model` is required.
    pub pk_section_existed: bool,

    // Moonshine
    pub mn_model: String,
    pub mn_quantized: bool,
    pub mn_threads: Option<i64>,
    pub mn_on_demand_loading: bool,
    pub mn_section_existed: bool,

    // SenseVoice
    pub sv_model: String,
    pub sv_language: String,
    pub sv_use_itn: bool,
    pub sv_threads: Option<i64>,
    pub sv_on_demand_loading: bool,
    pub sv_section_existed: bool,

    // Paraformer
    pub pf_model: String,
    pub pf_threads: Option<i64>,
    pub pf_on_demand_loading: bool,
    pub pf_section_existed: bool,

    // Dolphin
    pub dol_model: String,
    pub dol_threads: Option<i64>,
    pub dol_on_demand_loading: bool,
    pub dol_section_existed: bool,

    // Omnilingual
    pub om_model: String,
    pub om_threads: Option<i64>,
    pub om_on_demand_loading: bool,
    pub om_section_existed: bool,

    // Cohere Transcribe (ONNX, ~3 GB int8 model)
    pub co_model: String,
    pub co_language: String,
    pub co_threads: Option<i64>,
    pub co_on_demand_loading: bool,
    pub co_section_existed: bool,

    // OpenVINO GenAI Whisper (Intel NPU/GPU/CPU)
    pub ov_model: String,
    pub ov_device: String,
    pub ov_language: String,
    pub ov_threads: Option<i64>,
    pub ov_on_demand_loading: bool,
    pub ov_section_existed: bool,
}

#[derive(Debug, Clone)]
pub struct Feedback {
    pub level: FeedbackLevel,
    pub message: String,
}
#[derive(Debug, Clone, Copy)]
pub enum FeedbackLevel {
    Ok,
    Err,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldId {
    Engine,

    // Whisper
    WModel,
    WMode,
    WLanguage,
    WTranslate,
    WThreads,
    WPrompt,
    WFlashAttention,
    WOnDemandLoading,
    WGpuIsolation,
    WRemoteEndpoint,
    WRemoteApiKey,
    WRemoteModel,

    // Parakeet
    PkModel,
    PkModelType,
    PkOnDemandLoading,

    // Moonshine
    MnModel,
    MnQuantized,
    MnThreads,
    MnOnDemandLoading,

    // SenseVoice
    SvModel,
    SvLanguage,
    SvUseItn,
    SvThreads,
    SvOnDemandLoading,

    // Paraformer
    PfModel,
    PfThreads,
    PfOnDemandLoading,

    // Dolphin
    DolModel,
    DolThreads,
    DolOnDemandLoading,

    // Omnilingual
    OmModel,
    OmThreads,
    OmOnDemandLoading,

    // Cohere
    CoModel,
    CoLanguage,
    CoThreads,
    CoOnDemandLoading,

    // OpenVINO
    OvModel,
    OvDevice,
    OvLanguage,
    OvThreads,
    OvOnDemandLoading,
}

/// Cohere Transcribe officially supports these 14 languages. Token IDs are
/// looked up by name from tokens.txt at runtime, so the TUI only needs to
/// pass the two-letter code through to [cohere] language.
const CO_LANG_CHOICES: &[&str] = &[
    "ar", "de", "en", "es", "fr", "hi", "it", "ja", "ko", "nl", "pt", "ru", "tr", "zh",
];

const MODE_CHOICES: &[&str] = &["local", "remote", "cli"];
const LANG_CHOICES: &[&str] = &[
    "auto", "en", "fr", "de", "it", "es", "pt", "nl", "pl", "zh", "ja", "ko", "ru", "ar",
];
const SV_LANG_CHOICES: &[&str] = &["auto", "zh", "en", "ja", "ko", "yue"];
const PARAKEET_MODEL_TYPES: &[Option<&str>] = &[None, Some("tdt"), Some("ctc")];
/// Inference device OpenVINO GenAI tries first; falls back to CPU if the
/// requested device fails to initialize (see `OpenVinoTranscriber::new`).
/// AUTO is a real OpenVINO device value too (see `installation_guidance`'s
/// own AUTO arm) — included here so it's reachable from the picker.
const OV_DEVICE_CHOICES: &[&str] = &["NPU", "GPU", "CPU", "AUTO"];

fn rows_for_engine_with_mode(engine: &str, whisper_mode: &str) -> Vec<FieldId> {
    let mut rows = vec![FieldId::Engine];
    match engine {
        "whisper" => {
            rows.extend_from_slice(&[
                FieldId::WModel,
                FieldId::WMode,
                FieldId::WLanguage,
                FieldId::WTranslate,
                FieldId::WThreads,
                FieldId::WPrompt,
                FieldId::WFlashAttention,
                FieldId::WOnDemandLoading,
                FieldId::WGpuIsolation,
            ]);
            if whisper_mode == "remote" {
                rows.extend_from_slice(&[
                    FieldId::WRemoteEndpoint,
                    FieldId::WRemoteApiKey,
                    FieldId::WRemoteModel,
                ]);
            }
        }
        "parakeet" => rows.extend_from_slice(&[
            FieldId::PkModel,
            FieldId::PkModelType,
            FieldId::PkOnDemandLoading,
        ]),
        "moonshine" => rows.extend_from_slice(&[
            FieldId::MnModel,
            FieldId::MnQuantized,
            FieldId::MnThreads,
            FieldId::MnOnDemandLoading,
        ]),
        "sensevoice" => rows.extend_from_slice(&[
            FieldId::SvModel,
            FieldId::SvLanguage,
            FieldId::SvUseItn,
            FieldId::SvThreads,
            FieldId::SvOnDemandLoading,
        ]),
        "paraformer" => rows.extend_from_slice(&[
            FieldId::PfModel,
            FieldId::PfThreads,
            FieldId::PfOnDemandLoading,
        ]),
        "dolphin" => rows.extend_from_slice(&[
            FieldId::DolModel,
            FieldId::DolThreads,
            FieldId::DolOnDemandLoading,
        ]),
        "omnilingual" => rows.extend_from_slice(&[
            FieldId::OmModel,
            FieldId::OmThreads,
            FieldId::OmOnDemandLoading,
        ]),
        "cohere" => rows.extend_from_slice(&[
            FieldId::CoModel,
            FieldId::CoLanguage,
            FieldId::CoThreads,
            FieldId::CoOnDemandLoading,
        ]),
        "openvino" => rows.extend_from_slice(&[
            FieldId::OvModel,
            FieldId::OvDevice,
            FieldId::OvLanguage,
            FieldId::OvThreads,
            FieldId::OvOnDemandLoading,
        ]),
        _ => {}
    }
    rows
}

impl EngineState {
    pub fn load() -> Result<Self, EditorError> {
        let ed = ConfigEditor::load()?;
        let engine = ed
            .get_string("", "engine")
            .unwrap_or_else(|| "whisper".to_string());
        let fields = AllFields {
            // Whisper
            w_model: ed
                .get_string("whisper", "model")
                .unwrap_or_else(|| default_model("whisper").to_string()),
            w_mode: ed
                .get_string("whisper", "mode")
                .unwrap_or_else(|| "local".to_string()),
            w_language: ed
                .get_string("whisper", "language")
                .unwrap_or_else(|| "Automatic".to_string()),
            w_translate: ed.get_bool("whisper", "translate").unwrap_or(false),
            w_threads: ed.get_int("whisper", "threads"),
            w_initial_prompt: ed.get_string("whisper", "initial_prompt"),
            w_flash_attention: ed.get_bool("whisper", "flash_attention").unwrap_or(false),
            w_on_demand_loading: ed.get_bool("whisper", "on_demand_loading").unwrap_or(false),
            w_gpu_isolation: ed.get_bool("whisper", "gpu_isolation").unwrap_or(false),
            w_remote_endpoint: ed.get_string("whisper", "remote_endpoint"),
            w_remote_api_key: ed.get_string("whisper", "remote_api_key"),
            w_remote_model: ed.get_string("whisper", "remote_model"),

            // Parakeet
            pk_model: ed
                .get_string("parakeet", "model")
                .unwrap_or_else(|| default_model("parakeet").to_string()),
            pk_model_type: ed.get_string("parakeet", "model_type"),
            pk_on_demand_loading: ed
                .get_bool("parakeet", "on_demand_loading")
                .unwrap_or(false),
            pk_section_existed: ed.get_string("parakeet", "model").is_some(),

            // Moonshine
            mn_model: ed
                .get_string("moonshine", "model")
                .unwrap_or_else(|| default_model("moonshine").to_string()),
            mn_quantized: ed.get_bool("moonshine", "quantized").unwrap_or(true),
            mn_threads: ed.get_int("moonshine", "threads"),
            mn_on_demand_loading: ed
                .get_bool("moonshine", "on_demand_loading")
                .unwrap_or(false),
            mn_section_existed: ed.get_string("moonshine", "model").is_some(),

            // SenseVoice
            sv_model: ed
                .get_string("sensevoice", "model")
                .unwrap_or_else(|| default_model("sensevoice").to_string()),
            sv_language: ed
                .get_string("sensevoice", "language")
                .unwrap_or_else(|| "Automatic".to_string()),
            sv_use_itn: ed.get_bool("sensevoice", "use_itn").unwrap_or(true),
            sv_threads: ed.get_int("sensevoice", "threads"),
            sv_on_demand_loading: ed
                .get_bool("sensevoice", "on_demand_loading")
                .unwrap_or(false),
            sv_section_existed: ed.get_string("sensevoice", "model").is_some(),

            // Paraformer
            pf_model: ed
                .get_string("paraformer", "model")
                .unwrap_or_else(|| default_model("paraformer").to_string()),
            pf_threads: ed.get_int("paraformer", "threads"),
            pf_on_demand_loading: ed
                .get_bool("paraformer", "on_demand_loading")
                .unwrap_or(false),
            pf_section_existed: ed.get_string("paraformer", "model").is_some(),

            // Dolphin
            dol_model: ed
                .get_string("dolphin", "model")
                .unwrap_or_else(|| default_model("dolphin").to_string()),
            dol_threads: ed.get_int("dolphin", "threads"),
            dol_on_demand_loading: ed.get_bool("dolphin", "on_demand_loading").unwrap_or(false),
            dol_section_existed: ed.get_string("dolphin", "model").is_some(),

            // Omnilingual
            om_model: ed
                .get_string("omnilingual", "model")
                .unwrap_or_else(|| default_model("omnilingual").to_string()),
            om_threads: ed.get_int("omnilingual", "threads"),
            om_on_demand_loading: ed
                .get_bool("omnilingual", "on_demand_loading")
                .unwrap_or(false),
            om_section_existed: ed.get_string("omnilingual", "model").is_some(),

            // Cohere
            co_model: ed
                .get_string("cohere", "model")
                .unwrap_or_else(|| default_model("cohere").to_string()),
            co_language: ed
                .get_string("cohere", "language")
                .unwrap_or_else(|| "en".to_string()),
            co_threads: ed.get_int("cohere", "threads"),
            co_on_demand_loading: ed.get_bool("cohere", "on_demand_loading").unwrap_or(false),
            co_section_existed: ed.get_string("cohere", "model").is_some(),

            // OpenVINO
            ov_model: ed
                .get_string("openvino", "model")
                .unwrap_or_else(|| default_model("openvino").to_string()),
            ov_device: ed
                .get_string("openvino", "device")
                .unwrap_or_else(|| "NPU".to_string()),
            ov_language: ed
                .get_string("openvino", "language")
                .unwrap_or_else(|| "en".to_string()),
            ov_threads: ed.get_int("openvino", "threads"),
            ov_on_demand_loading: ed
                .get_bool("openvino", "on_demand_loading")
                .unwrap_or(false),
            ov_section_existed: ed.get_string("openvino", "model").is_some(),
        };
        let mut state = Self {
            engine,
            fields,
            cursor: 0,
            feedback: None,
            dirty_since_load: false,
            pending_variant_switch: None,
            binary_switch_blocked: None,
            editing: None,
        };
        state.refresh_binary_match();
        Ok(state)
    }

    /// Required binary family for an engine name. Whisper needs a Whisper
    /// binary; everything ONNX-based needs an ONNX binary.
    fn required_family(engine: &str) -> EngineFamily {
        if engine == "whisper" {
            EngineFamily::Whisper
        } else {
            EngineFamily::Onnx
        }
    }

    /// Recompute pending_variant_switch / binary_switch_blocked based on the
    /// current engine selection. Called whenever the engine field changes.
    fn refresh_binary_match(&mut self) {
        self.pending_variant_switch = None;
        self.binary_switch_blocked = None;

        let inv = binary::inventory();

        // OpenVINO isn't part of the prebuilt binary distribution: no
        // Whisper or ONNX variant links against openvino-genai (it's an
        // entirely separate Intel runtime, not an ONNX execution provider),
        // so there's no variant to switch to. It's purely a source-build
        // Cargo feature; just check whether the running binary has it.
        if self.engine == "openvino" {
            if !inv.compiled_features.contains(&"openvino") {
                self.binary_switch_blocked = Some(
                    "No prebuilt voxtype includes OpenVINO. Build it from source: \
                     cargo build --release --features openvino"
                        .to_string(),
                );
            }
            return;
        }

        if inv.install_kind == InstallKind::Source {
            // Source builds can't be hot-swapped; whether the running binary
            // supports the chosen engine depends on its compiled features.
            // Best we can do is flag it.
            let supported = match self.engine.as_str() {
                "whisper" => true,
                "parakeet" => inv.compiled_features.contains(&"parakeet"),
                _ => inv.compiled_features.contains(&self.engine.as_str()),
            };
            if !supported {
                self.binary_switch_blocked = Some(format!(
                    "This voxtype was built without {}. Rebuild it with: \
                     cargo build --release --features {}",
                    display_engine(&self.engine),
                    self.engine
                ));
            }
            return;
        }

        let needed = Self::required_family(&self.engine);
        let current_family = inv.active_variant.map(|v| v.family());
        if current_family == Some(needed) {
            return; // already matches
        }

        // Pick the recommended variant for the needed family on this hardware.
        let target = if needed == EngineFamily::Whisper {
            inv.recommendation.whisper
        } else {
            inv.recommendation.onnx
        };

        // Confirm the recommended variant is actually installed and runnable.
        let runnable = inv
            .variants
            .iter()
            .find(|s| s.variant == target)
            .map(|s| s.installed && s.runs_on_this_cpu && s.gpu_available)
            .unwrap_or(false);

        if runnable {
            self.pending_variant_switch = Some(target);
        } else {
            // Fall back to any installed variant of the right family that
            // runs on this hardware.
            let fallback = inv.variants.iter().find(|s| {
                s.variant.family() == needed && s.installed && s.runs_on_this_cpu && s.gpu_available
            });
            match fallback {
                Some(s) => self.pending_variant_switch = Some(s.variant),
                None => {
                    self.binary_switch_blocked = Some(format!(
                        "No installed voxtype build can run {} on this machine. \
                         See the Build page.",
                        display_engine(&self.engine)
                    ));
                }
            }
        }
    }

    pub fn save(&mut self) -> Action {
        let mut ed = match ConfigEditor::load() {
            Ok(e) => e,
            Err(e) => {
                self.feedback = Some(Feedback {
                    level: FeedbackLevel::Err,
                    message: format!("Couldn't read the config: {}", e),
                });
                return Action::None;
            }
        };

        // Active engine at root.
        ed.set_string("", "engine", &self.engine);

        let f = &self.fields;

        // Whisper — always written; voxtype assumes a Whisper config exists.
        ed.set_string("whisper", "model", &f.w_model);
        ed.set_string("whisper", "mode", &f.w_mode);
        ed.set_string("whisper", "language", &f.w_language);
        ed.set_bool("whisper", "translate", f.w_translate);
        match f.w_threads {
            Some(n) => ed.set_int("whisper", "threads", n),
            None => ed.unset("whisper", "threads"),
        }
        match &f.w_initial_prompt {
            Some(p) if !p.is_empty() => ed.set_string("whisper", "initial_prompt", p),
            _ => ed.unset("whisper", "initial_prompt"),
        }
        ed.set_bool("whisper", "flash_attention", f.w_flash_attention);
        ed.set_bool("whisper", "on_demand_loading", f.w_on_demand_loading);
        ed.set_bool("whisper", "gpu_isolation", f.w_gpu_isolation);
        match &f.w_remote_endpoint {
            Some(v) if !v.is_empty() => ed.set_string("whisper", "remote_endpoint", v),
            _ => ed.unset("whisper", "remote_endpoint"),
        }
        match &f.w_remote_api_key {
            Some(v) if !v.is_empty() => ed.set_string("whisper", "remote_api_key", v),
            _ => ed.unset("whisper", "remote_api_key"),
        }
        match &f.w_remote_model {
            Some(v) if !v.is_empty() => ed.set_string("whisper", "remote_model", v),
            _ => ed.unset("whisper", "remote_model"),
        }

        // Parakeet — only touch the table if it already existed or the user
        // is making it the active engine. Now that the user can edit model
        // here directly, the model field is always written when we touch the
        // table.
        if self.engine == "parakeet" || f.pk_section_existed {
            ed.set_string("parakeet", "model", &f.pk_model);
            match &f.pk_model_type {
                Some(m) => ed.set_string("parakeet", "model_type", m),
                None => ed.unset("parakeet", "model_type"),
            }
            ed.set_bool("parakeet", "on_demand_loading", f.pk_on_demand_loading);
        }

        // Moonshine
        if self.engine == "moonshine" || f.mn_section_existed {
            ed.set_string("moonshine", "model", &f.mn_model);
            ed.set_bool("moonshine", "quantized", f.mn_quantized);
            match f.mn_threads {
                Some(n) => ed.set_int("moonshine", "threads", n),
                None => ed.unset("moonshine", "threads"),
            }
            ed.set_bool("moonshine", "on_demand_loading", f.mn_on_demand_loading);
        }

        // SenseVoice
        if self.engine == "sensevoice" || f.sv_section_existed {
            ed.set_string("sensevoice", "model", &f.sv_model);
            ed.set_string("sensevoice", "language", &f.sv_language);
            ed.set_bool("sensevoice", "use_itn", f.sv_use_itn);
            match f.sv_threads {
                Some(n) => ed.set_int("sensevoice", "threads", n),
                None => ed.unset("sensevoice", "threads"),
            }
            ed.set_bool("sensevoice", "on_demand_loading", f.sv_on_demand_loading);
        }

        // Paraformer
        if self.engine == "paraformer" || f.pf_section_existed {
            ed.set_string("paraformer", "model", &f.pf_model);
            match f.pf_threads {
                Some(n) => ed.set_int("paraformer", "threads", n),
                None => ed.unset("paraformer", "threads"),
            }
            ed.set_bool("paraformer", "on_demand_loading", f.pf_on_demand_loading);
        }

        // Dolphin
        if self.engine == "dolphin" || f.dol_section_existed {
            ed.set_string("dolphin", "model", &f.dol_model);
            match f.dol_threads {
                Some(n) => ed.set_int("dolphin", "threads", n),
                None => ed.unset("dolphin", "threads"),
            }
            ed.set_bool("dolphin", "on_demand_loading", f.dol_on_demand_loading);
        }

        // Omnilingual
        if self.engine == "omnilingual" || f.om_section_existed {
            ed.set_string("omnilingual", "model", &f.om_model);
            match f.om_threads {
                Some(n) => ed.set_int("omnilingual", "threads", n),
                None => ed.unset("omnilingual", "threads"),
            }
            ed.set_bool("omnilingual", "on_demand_loading", f.om_on_demand_loading);
        }

        // Cohere
        if self.engine == "cohere" || f.co_section_existed {
            ed.set_string("cohere", "model", &f.co_model);
            ed.set_string("cohere", "language", &f.co_language);
            match f.co_threads {
                Some(n) => ed.set_int("cohere", "threads", n),
                None => ed.unset("cohere", "threads"),
            }
            ed.set_bool("cohere", "on_demand_loading", f.co_on_demand_loading);
        }

        // OpenVINO
        if self.engine == "openvino" || f.ov_section_existed {
            ed.set_string("openvino", "model", &f.ov_model);
            ed.set_string("openvino", "device", &f.ov_device);
            ed.set_string("openvino", "language", &f.ov_language);
            match f.ov_threads {
                Some(n) => ed.set_int("openvino", "threads", n),
                None => ed.unset("openvino", "threads"),
            }
            ed.set_bool("openvino", "on_demand_loading", f.ov_on_demand_loading);
        }

        match ed.save() {
            Ok(()) => {
                self.dirty_since_load = false;
                let pending_switch = self.pending_variant_switch.take();
                let active_model = active_model_for_engine(&self.engine, &self.fields);
                let download_needed = active_model
                    .as_deref()
                    .map(|m| !model_present_on_disk(&self.engine, m))
                    .unwrap_or(false);

                let next_action = match (pending_switch, download_needed, active_model.clone()) {
                    (Some(v), true, Some(model)) => Action::SwitchVariantAndDownload {
                        variant: v,
                        engine: self.engine.clone(),
                        model,
                    },
                    (Some(v), false, _) => Action::SwitchVariant(v),
                    (None, true, Some(model)) => Action::DownloadModel {
                        engine: self.engine.clone(),
                        model,
                    },
                    _ => Action::None,
                };

                self.feedback = Some(Feedback {
                    level: FeedbackLevel::Ok,
                    message: match &next_action {
                        Action::SwitchVariantAndDownload { variant, model, .. } => format!(
                            "Saved. Switching to {} (sudo) and downloading `{}`…",
                            variant.display(),
                            model
                        ),
                        Action::SwitchVariant(v) => format!(
                            "Saved. Switching binary to {} (will prompt for sudo)…",
                            v.display()
                        ),
                        Action::DownloadModel { model, .. } => {
                            format!("Saved. Downloading model `{}`…", model)
                        }
                        _ => format!("Saved to {}", ed.path().display()),
                    },
                });

                return next_action;
            }
            Err(e) => {
                self.feedback = Some(Feedback {
                    level: FeedbackLevel::Err,
                    message: format!("Not saved: {}", e),
                });
            }
        }
        Action::None
    }

    pub fn reset(&mut self) {
        match Self::load() {
            Ok(fresh) => {
                let cursor = self.cursor;
                *self = fresh;
                let max = self.rows().len().saturating_sub(1);
                self.cursor = cursor.min(max);
                self.refresh_binary_match();
                self.feedback = Some(Feedback {
                    level: FeedbackLevel::Ok,
                    message: "Put back the saved settings".to_string(),
                });
            }
            Err(e) => {
                self.feedback = Some(Feedback {
                    level: FeedbackLevel::Err,
                    message: format!("Couldn't read the config: {}", e),
                });
            }
        }
    }

    fn move_cursor(&mut self, delta: i32) {
        let len = self.rows().len() as i32;
        if len == 0 {
            return;
        }
        let new = (self.cursor as i32 + delta).rem_euclid(len);
        self.cursor = new as usize;
    }

    /// Visible rows for the current engine. Whisper has extra rows when
    /// running in remote mode; everything else is constant per engine.
    fn rows(&self) -> Vec<FieldId> {
        rows_for_engine_with_mode(&self.engine, &self.fields.w_mode)
    }

    fn current_field(&self) -> FieldId {
        let rows = self.rows();
        rows.get(self.cursor).copied().unwrap_or(FieldId::Engine)
    }

    /// True if the focused field is a free-text field that should be edited
    /// with the inline TextInput rather than a cycle list.
    fn is_text_field(field: FieldId) -> bool {
        matches!(
            field,
            FieldId::WPrompt
                | FieldId::WRemoteEndpoint
                | FieldId::WRemoteApiKey
                | FieldId::WRemoteModel
        )
    }

    fn start_edit_if_text_field(&mut self) -> bool {
        let field = self.current_field();
        if !Self::is_text_field(field) {
            return false;
        }
        let initial = match field {
            FieldId::WPrompt => self.fields.w_initial_prompt.clone().unwrap_or_default(),
            FieldId::WRemoteEndpoint => self.fields.w_remote_endpoint.clone().unwrap_or_default(),
            FieldId::WRemoteApiKey => self.fields.w_remote_api_key.clone().unwrap_or_default(),
            FieldId::WRemoteModel => self.fields.w_remote_model.clone().unwrap_or_default(),
            _ => String::new(),
        };
        self.editing = Some(TextEdit {
            field,
            input: TextInput::new(initial),
        });
        true
    }

    fn commit_text_edit(&mut self, field: FieldId, buffer: String) {
        let trimmed = buffer.trim();
        let opt = if trimmed.is_empty() {
            None
        } else {
            Some(buffer.clone())
        };
        match field {
            FieldId::WPrompt => self.fields.w_initial_prompt = opt,
            FieldId::WRemoteEndpoint => self.fields.w_remote_endpoint = opt,
            FieldId::WRemoteApiKey => self.fields.w_remote_api_key = opt,
            FieldId::WRemoteModel => self.fields.w_remote_model = opt,
            _ => {}
        }
        self.dirty_since_load = true;
        self.feedback = None;
    }

    fn cycle(&mut self, delta: i32) {
        let field = self.current_field();
        let f = &mut self.fields;
        match field {
            FieldId::Engine => {
                // Filter to engines for which at least one installed binary
                // can be the active variant. On a tiered install this means
                // /usr/bin/voxtype can be symlinked to a binary that has the
                // matching feature; on a source build it means the running
                // binary was compiled with that feature flag.
                let installed = installed_engine_choices();
                let filtered: Vec<&'static str> = TranscriptionEngine::iter()
                    .map(|e| e.name())
                    .filter(|e| installed.contains(e))
                    .collect();
                if filtered.is_empty() {
                    // Nothing to cycle through; leave the engine unchanged.
                    return;
                }
                let idx = filtered
                    .iter()
                    .position(|c| *c == self.engine)
                    .map(|i| i as i32)
                    .unwrap_or(0);
                let n = (idx + delta).rem_euclid(filtered.len() as i32);
                self.engine = filtered[n as usize].to_string();
                let max = self.rows().len().saturating_sub(1);
                self.cursor = self.cursor.min(max);
                self.refresh_binary_match();
            }
            FieldId::WModel => f.w_model = cycle_model("whisper", &f.w_model, delta),
            FieldId::WMode => f.w_mode = cycle_str(MODE_CHOICES, &f.w_mode, delta),
            FieldId::WLanguage => f.w_language = cycle_str(LANG_CHOICES, &f.w_language, delta),
            FieldId::WTranslate => f.w_translate = !f.w_translate,
            FieldId::WThreads => f.w_threads = cycle_threads(f.w_threads, delta),
            FieldId::WPrompt => {
                // Free-text field: enter inline edit mode instead of cycling
                // through hardcoded presets.
                self.editing = Some(TextEdit {
                    field: FieldId::WPrompt,
                    input: TextInput::new(f.w_initial_prompt.clone().unwrap_or_default()),
                });
                return; // Don't mark dirty until commit.
            }
            FieldId::WFlashAttention => f.w_flash_attention = !f.w_flash_attention,
            FieldId::WOnDemandLoading => f.w_on_demand_loading = !f.w_on_demand_loading,
            FieldId::WGpuIsolation => f.w_gpu_isolation = !f.w_gpu_isolation,
            FieldId::WRemoteEndpoint | FieldId::WRemoteApiKey | FieldId::WRemoteModel => {
                self.start_edit_if_text_field();
                return;
            }

            FieldId::PkModel => f.pk_model = cycle_model("parakeet", &f.pk_model, delta),
            FieldId::PkModelType => {
                let idx = PARAKEET_MODEL_TYPES
                    .iter()
                    .position(|c| c.as_deref() == f.pk_model_type.as_deref())
                    .map(|i| i as i32)
                    .unwrap_or(0);
                let n = (idx + delta).rem_euclid(PARAKEET_MODEL_TYPES.len() as i32);
                f.pk_model_type = PARAKEET_MODEL_TYPES[n as usize].map(|s| s.to_string());
            }
            FieldId::PkOnDemandLoading => f.pk_on_demand_loading = !f.pk_on_demand_loading,

            FieldId::MnModel => f.mn_model = cycle_model("moonshine", &f.mn_model, delta),
            FieldId::MnQuantized => f.mn_quantized = !f.mn_quantized,
            FieldId::MnThreads => f.mn_threads = cycle_threads(f.mn_threads, delta),
            FieldId::MnOnDemandLoading => f.mn_on_demand_loading = !f.mn_on_demand_loading,

            FieldId::SvModel => f.sv_model = cycle_model("sensevoice", &f.sv_model, delta),
            FieldId::SvLanguage => {
                f.sv_language = cycle_str(SV_LANG_CHOICES, &f.sv_language, delta)
            }
            FieldId::SvUseItn => f.sv_use_itn = !f.sv_use_itn,
            FieldId::SvThreads => f.sv_threads = cycle_threads(f.sv_threads, delta),
            FieldId::SvOnDemandLoading => f.sv_on_demand_loading = !f.sv_on_demand_loading,

            FieldId::PfModel => f.pf_model = cycle_model("paraformer", &f.pf_model, delta),
            FieldId::PfThreads => f.pf_threads = cycle_threads(f.pf_threads, delta),
            FieldId::PfOnDemandLoading => f.pf_on_demand_loading = !f.pf_on_demand_loading,

            FieldId::DolModel => f.dol_model = cycle_model("dolphin", &f.dol_model, delta),
            FieldId::DolThreads => f.dol_threads = cycle_threads(f.dol_threads, delta),
            FieldId::DolOnDemandLoading => f.dol_on_demand_loading = !f.dol_on_demand_loading,

            FieldId::OmModel => f.om_model = cycle_model("omnilingual", &f.om_model, delta),
            FieldId::OmThreads => f.om_threads = cycle_threads(f.om_threads, delta),
            FieldId::OmOnDemandLoading => f.om_on_demand_loading = !f.om_on_demand_loading,

            FieldId::CoModel => f.co_model = cycle_model("cohere", &f.co_model, delta),
            FieldId::CoLanguage => {
                f.co_language = cycle_str(CO_LANG_CHOICES, &f.co_language, delta)
            }
            FieldId::CoThreads => f.co_threads = cycle_threads(f.co_threads, delta),
            FieldId::CoOnDemandLoading => f.co_on_demand_loading = !f.co_on_demand_loading,

            FieldId::OvModel => f.ov_model = cycle_model("openvino", &f.ov_model, delta),
            FieldId::OvDevice => f.ov_device = cycle_str(OV_DEVICE_CHOICES, &f.ov_device, delta),
            FieldId::OvLanguage => f.ov_language = cycle_str(LANG_CHOICES, &f.ov_language, delta),
            FieldId::OvThreads => f.ov_threads = cycle_threads(f.ov_threads, delta),
            FieldId::OvOnDemandLoading => f.ov_on_demand_loading = !f.ov_on_demand_loading,
        }
        self.dirty_since_load = true;
        self.feedback = None;
    }
}

/// The model name for the active engine — what the daemon will try to load
/// at startup. Used to decide whether the engine save needs to trigger a
/// model download alongside (or instead of) a binary swap. The daemon
/// doesn't download a missing Whisper model either, so Whisper is checked
/// like the rest. Returns None for unknown engines.
fn active_model_for_engine(engine: &str, f: &AllFields) -> Option<String> {
    match engine {
        // A path names a file the user manages; a remote Whisper has no
        // local model at all.
        "whisper" if f.w_mode == "remote" || f.w_model.contains('/') => None,
        "whisper" => Some(f.w_model.clone()),
        "parakeet" => Some(f.pk_model.clone()),
        "moonshine" => Some(f.mn_model.clone()),
        "sensevoice" => Some(f.sv_model.clone()),
        "paraformer" => Some(f.pf_model.clone()),
        "dolphin" => Some(f.dol_model.clone()),
        "omnilingual" => Some(f.om_model.clone()),
        "cohere" => Some(f.co_model.clone()),
        "openvino" => Some(f.ov_model.clone()),
        _ => None,
    }
}

/// Engines (by name) the user can land on in the Engine cycle. Engines whose
/// binary is not installed are excluded so users can't pick one and then have
/// the daemon refuse to start.
///
/// We always check /usr/lib/voxtype/ first via `installed_engines(&inv)`,
/// regardless of whether the running TUI is a package binary or a source
/// build — the daemon runs whatever /usr/bin/voxtype resolves to, not
/// necessarily the same binary as the TUI. A developer running a source-built
/// `voxtype configure` on a host that also has the AUR package installed
/// should still see every engine the package binaries support.
///
/// The compiled-features fallback only kicks in for pure source builds with
/// no package binaries installed alongside (CI machines, fresh dev VMs);
/// there, the daemon would have to run the same source binary, so Cargo
/// features are the right gate.
fn installed_engine_choices() -> std::collections::HashSet<&'static str> {
    let mut out = std::collections::HashSet::new();

    // Look at /usr/lib/voxtype/ first. enumerate_installed walks the dir
    // directly, independent of install_kind, so a source build running
    // alongside an AUR install still discovers package binaries.
    for variant in binary::enumerate_installed() {
        for engine in [
            "whisper",
            "parakeet",
            "moonshine",
            "sensevoice",
            "paraformer",
            "dolphin",
            "omnilingual",
            "cohere",
            "openvino",
        ] {
            if variant.supports_engine(engine) {
                out.insert(engine);
            }
        }
    }

    // Also include engines the running binary itself was compiled with —
    // covers pure source builds with no /usr/lib/voxtype/. The
    // no-features cargo default still ships the Whisper engine, so
    // Whisper is always present.
    let inv = binary::inventory();
    out.insert("whisper");
    for f in &inv.compiled_features {
        for engine in [
            "parakeet",
            "moonshine",
            "sensevoice",
            "paraformer",
            "dolphin",
            "omnilingual",
            "cohere",
            // OpenVINO isn't checked against `variant.supports_engine()`
            // above — it has no prebuilt binary variant at all (Intel-only
            // runtime, unrelated to the Whisper/ONNX distribution). Source
            // builds with the feature compiled in are the only way it runs.
            "openvino",
        ] {
            if *f == engine {
                out.insert(engine);
            }
        }
    }

    out
}

/// Is the configured model usable, so a save does not need to download it?
///
/// Delegates to `model_catalog::model_installed`, which is what the daemon
/// and the downloader already resolve through. Asking it rather than probing
/// the directory ourselves buys two things.
///
/// It handles the name mapping: `name` is the raw config value, and for
/// moonshine and sensevoice that is a short name that differs from the
/// directory the downloader wrote to. Getting that wrong is what made a
/// working model report as absent (#662).
///
/// It also rejects a model that is present but damaged. The check here used
/// to be "the directory exists and has at least one file in it", which calls
/// an interrupted download complete and skips the re-download that would fix
/// it. `model_installed` verifies against the recorded manifest, so a partial
/// download is treated as needing a download.
fn model_present_on_disk(engine: &str, name: &str) -> bool {
    crate::model_catalog::model_installed(engine, name)
}

fn cycle_str(choices: &[&'static str], current: &str, delta: i32) -> String {
    if choices.is_empty() {
        return current.to_string();
    }
    let idx = choices
        .iter()
        .position(|c| *c == current)
        .map(|i| i as i32)
        .unwrap_or(-1);
    let new = (idx + delta).rem_euclid(choices.len() as i32);
    choices[new as usize].to_string()
}

fn cycle_model(engine: &str, current: &str, delta: i32) -> String {
    let names = model_catalog(engine);
    if names.is_empty() {
        return current.to_string();
    }
    let idx = names
        .iter()
        .position(|c| *c == current)
        .map(|i| i as i32)
        .unwrap_or(-1);
    let new = (idx + delta).rem_euclid(names.len() as i32);
    names[new as usize].to_string()
}

fn cycle_threads(current: Option<i64>, delta: i32) -> Option<i64> {
    let cur = current.unwrap_or(0);
    let next = cur + delta as i64;
    if next <= 0 {
        None
    } else {
        Some(next.min(64))
    }
}

/// The page's rows: the engine, then the active engine's own settings.
pub fn rows(state: &EngineState) -> Vec<FormRowSpec> {
    state
        .rows()
        .iter()
        .enumerate()
        .map(|(i, fid)| {
            let (label, value) = field_label_value(state, *fid);
            let row = FormRowSpec::new(i == state.cursor, label, value);
            match fid {
                FieldId::WMode => row.choices(&["Local", "Remote", "CLI"]),
                FieldId::PkModelType => row.choices(&["Automatic", "TDT", "CTC"]),
                FieldId::OvDevice => row.choices(OV_DEVICE_CHOICES),
                _ => row,
            }
        })
        .collect()
}

/// Put the cursor on the row labelled `label`, for Basics.
pub fn focus_label(state: &mut EngineState, label: &str) {
    if let Some(i) = rows(state).iter().position(|r| r.label == label) {
        state.cursor = i;
    }
}

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let Some(state) = &app.engine else {
        common::render_load_error(f, area, app);
        return;
    };
    let feedback_pair = state.feedback.as_ref().map(|fb| {
        (
            match fb.level {
                FeedbackLevel::Ok => CommonFeedback::Ok,
                FeedbackLevel::Err => CommonFeedback::Err,
            },
            fb.message.as_str(),
        )
    });
    common::render_form_with_guidance(f, area, app, feedback_pair, &rows(state), guidance(state));
}

/// Engines whose ONNX graphs MIGraphX 7.2 can't compile cleanly. On the
/// AMD-targeted binary they fall back to CPU even though the GPU is
/// registered for Parakeet. Surfaces a small `(CPU on AMD GPU)` tag in
/// the picker. Cohere is on this list because the int8 community export
/// uses MatMulNBits(bits=8) which MIGraphX 7.2 rejects; will move off
/// once an int4 / FP16 export ships.
const AMD_CPU_ONLY_ENGINES: &[&str] = &[
    "moonshine",
    "sensevoice",
    "paraformer",
    "dolphin",
    "omnilingual",
    "cohere",
];

fn engine_value_for_display(engine: &str) -> String {
    if cfg!(feature = "onnx-migraphx-enabled") && AMD_CPU_ONLY_ENGINES.contains(&engine) {
        format!("{}  (CPU on AMD GPU)", display_engine(engine))
    } else {
        display_engine(engine).to_string()
    }
}

fn field_label_value(state: &EngineState, fid: FieldId) -> (&'static str, String) {
    let f = &state.fields;
    match fid {
        FieldId::Engine => ("Engine", engine_value_for_display(&state.engine)),

        FieldId::WModel => ("Model", f.w_model.clone()),
        FieldId::WMode => ("Runs", capitalize_mode(&f.w_mode)),
        FieldId::WLanguage => ("Language", language_name(&f.w_language)),
        FieldId::WTranslate => ("Translate to English", yesno(f.w_translate)),
        FieldId::WThreads => (
            "Threads",
            f.w_threads
                .map(|n| n.to_string())
                .unwrap_or_else(|| "Automatic".to_string()),
        ),
        FieldId::WPrompt => (
            "Initial prompt",
            match state.editing.as_ref() {
                Some(e) if e.field == FieldId::WPrompt => e.input.caret_string(),
                _ => f
                    .w_initial_prompt
                    .as_deref()
                    .map(|s| {
                        if s.len() > 30 {
                            format!("{}…", &s[..30])
                        } else {
                            s.to_string()
                        }
                    })
                    .unwrap_or_else(|| common::NOT_SET.to_string()),
            },
        ),
        FieldId::WFlashAttention => ("Flash attention", yesno(f.w_flash_attention)),
        FieldId::WOnDemandLoading => ("Load model on demand", yesno(f.w_on_demand_loading)),
        FieldId::WGpuIsolation => ("GPU isolation", yesno(f.w_gpu_isolation)),
        FieldId::WRemoteEndpoint => (
            "Remote endpoint",
            match state.editing.as_ref() {
                Some(e) if e.field == FieldId::WRemoteEndpoint => e.input.caret_string(),
                _ => f
                    .w_remote_endpoint
                    .clone()
                    .unwrap_or_else(|| common::NOT_SET.to_string()),
            },
        ),
        FieldId::WRemoteApiKey => (
            "Remote API key",
            match state.editing.as_ref() {
                Some(e) if e.field == FieldId::WRemoteApiKey => mask(&e.input.caret_string()),
                _ => match f.w_remote_api_key.as_deref() {
                    None | Some("") => common::NOT_SET.to_string(),
                    Some(_) => "••••••".to_string(),
                },
            },
        ),
        FieldId::WRemoteModel => (
            "Remote model",
            match state.editing.as_ref() {
                Some(e) if e.field == FieldId::WRemoteModel => e.input.caret_string(),
                _ => f
                    .w_remote_model
                    .clone()
                    .unwrap_or_else(|| common::NOT_SET.to_string()),
            },
        ),

        FieldId::PkModel => {
            // Mark streaming-capable models in the picker so the user can tell
            // at a glance which ones work with [parakeet] streaming = true.
            // ParakeetUnified needs tokenizer.model to be present; only a
            // subset of registered Parakeet models ship it. See model::is_streaming_compatible_parakeet
            // and src/setup/model.rs's PARAKEET_MODELS for the source of truth.
            ("Model", f.pk_model.clone())
        }
        FieldId::PkModelType => (
            "Model type",
            match f.pk_model_type.as_deref() {
                Some("tdt") => "TDT".to_string(),
                Some("ctc") => "CTC".to_string(),
                Some(other) => other.to_string(),
                None => "Automatic".to_string(),
            },
        ),
        FieldId::PkOnDemandLoading => ("Load model on demand", yesno(f.pk_on_demand_loading)),

        FieldId::MnModel => ("Model", f.mn_model.clone()),
        FieldId::MnQuantized => ("Quantized model", yesno(f.mn_quantized)),
        FieldId::MnThreads => (
            "Threads",
            f.mn_threads
                .map(|n| n.to_string())
                .unwrap_or_else(|| "Automatic".to_string()),
        ),
        FieldId::MnOnDemandLoading => ("Load model on demand", yesno(f.mn_on_demand_loading)),

        FieldId::SvModel => ("Model", f.sv_model.clone()),
        FieldId::SvLanguage => ("Language", language_name(&f.sv_language)),
        FieldId::SvUseItn => ("Write numbers as digits", yesno(f.sv_use_itn)),
        FieldId::SvThreads => (
            "Threads",
            f.sv_threads
                .map(|n| n.to_string())
                .unwrap_or_else(|| "Automatic".to_string()),
        ),
        FieldId::SvOnDemandLoading => ("Load model on demand", yesno(f.sv_on_demand_loading)),

        FieldId::PfModel => ("Model", f.pf_model.clone()),
        FieldId::PfThreads => (
            "Threads",
            f.pf_threads
                .map(|n| n.to_string())
                .unwrap_or_else(|| "Automatic".to_string()),
        ),
        FieldId::PfOnDemandLoading => ("Load model on demand", yesno(f.pf_on_demand_loading)),

        FieldId::DolModel => ("Model", f.dol_model.clone()),
        FieldId::DolThreads => (
            "Threads",
            f.dol_threads
                .map(|n| n.to_string())
                .unwrap_or_else(|| "Automatic".to_string()),
        ),
        FieldId::DolOnDemandLoading => ("Load model on demand", yesno(f.dol_on_demand_loading)),

        FieldId::OmModel => ("Model", f.om_model.clone()),
        FieldId::OmThreads => (
            "Threads",
            f.om_threads
                .map(|n| n.to_string())
                .unwrap_or_else(|| "Automatic".to_string()),
        ),
        FieldId::OmOnDemandLoading => ("Load model on demand", yesno(f.om_on_demand_loading)),

        FieldId::CoModel => ("Model", f.co_model.clone()),
        FieldId::CoLanguage => ("Language", language_name(&f.co_language)),
        FieldId::CoThreads => (
            "Threads",
            f.co_threads
                .map(|n| n.to_string())
                .unwrap_or_else(|| "Automatic".to_string()),
        ),
        FieldId::CoOnDemandLoading => ("Load model on demand", yesno(f.co_on_demand_loading)),

        FieldId::OvModel => ("Model", f.ov_model.clone()),
        FieldId::OvDevice => ("Device", f.ov_device.clone()),
        FieldId::OvLanguage => ("Language", language_name(&f.ov_language)),
        FieldId::OvThreads => (
            "Threads",
            f.ov_threads
                .map(|n| n.to_string())
                .unwrap_or_else(|| "Automatic".to_string()),
        ),
        FieldId::OvOnDemandLoading => ("Load model on demand", yesno(f.ov_on_demand_loading)),
    }
}

fn yesno(b: bool) -> String {
    common::on_off(b)
}

/// Mask a secret value for on-screen display while editing — show only the
/// final character so the user can verify they're typing what they intended,
/// but don't render the full key in the form row.
fn mask(s: &str) -> String {
    if s.is_empty() {
        return s.to_string();
    }
    let last = s.chars().last().map(|c| c.to_string()).unwrap_or_default();
    let bullets: String = std::iter::repeat_n('•', s.chars().count() - 1).collect();
    format!("{}{}", bullets, last)
}

/// One sentence on what an engine is good at.
pub fn engine_summary(engine: &str) -> &'static str {
    match engine {
        "whisper" => "Whisper is the default: 99 languages and good accuracy everywhere.",
        "parakeet" => {
            "Parakeet is fast and among the most accurate for English and 25 European languages."
        }
        "moonshine" => "Moonshine is small and quick, made for English dictation.",
        "sensevoice" => "SenseVoice is strong on Chinese, Japanese, Korean, Cantonese and English.",
        "paraformer" => "Paraformer is made for Chinese.",
        "dolphin" => "Dolphin is tuned for dictation in Eastern languages.",
        "omnilingual" => "Omnilingual covers more than 1,600 languages.",
        "cohere" => "Cohere Transcribe is very accurate in 14 languages, and large (about 3 GB).",
        "openvino" => "OpenVINO runs Whisper on Intel NPUs, GPUs and CPUs.",
        _ => "",
    }
}

/// What saving this engine will also do: switch build, or why it can't.
pub fn engine_switch_note(state: &EngineState) -> Option<(bool, String)> {
    if let Some(target) = state.pending_variant_switch {
        Some((
            false,
            format!(
                "Saving also switches voxtype to the {} build. It asks for your password.",
                super::build::variant_name(target)
            ),
        ))
    } else {
        state
            .binary_switch_blocked
            .clone()
            .map(|reason| (true, reason))
    }
}

fn engine_guidance(state: &EngineState) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(engine_summary(&state.engine).to_string())];
    if let Some((blocked, note)) = engine_switch_note(state) {
        lines.push(Line::from(Span::styled(
            note,
            if blocked {
                theme::error()
            } else {
                theme::warn()
            },
        )));
    }
    lines.push(Line::from(Span::styled(
        "←→ goes through the engines this install can run.",
        theme::muted(),
    )));
    lines
}

fn guidance(state: &EngineState) -> Vec<Line<'_>> {
    let f = &state.fields;
    let text = |s: &'static str| Line::from(s);
    let dim = |s: &'static str| Line::from(Span::styled(s, theme::muted()));
    match state.current_field() {
        FieldId::Engine => engine_guidance(state),

        FieldId::WModel => model_guidance("whisper", &f.w_model),
        FieldId::PkModel => model_guidance("parakeet", &f.pk_model),
        FieldId::MnModel => model_guidance("moonshine", &f.mn_model),
        FieldId::SvModel => model_guidance("sensevoice", &f.sv_model),
        FieldId::PfModel => model_guidance("paraformer", &f.pf_model),
        FieldId::DolModel => model_guidance("dolphin", &f.dol_model),
        FieldId::OmModel => model_guidance("omnilingual", &f.om_model),
        FieldId::CoModel => model_guidance("cohere", &f.co_model),
        FieldId::OvModel => model_guidance("openvino", &f.ov_model),

        FieldId::WMode => vec![text(
            "Local runs Whisper on this computer. Remote sends audio to an \
             OpenAI-compatible server, set below. CLI runs a whisper-cli program \
             instead of the built-in one.",
        )],
        FieldId::WLanguage | FieldId::OvLanguage => vec![
            text(
                "The language you dictate in. Automatic detects it each time; \
                 choosing one is a little faster and more reliable.",
            ),
            dim("Models ending in .en only do English."),
        ],
        FieldId::WTranslate => vec![text("Writes English whatever language you speak.")],
        FieldId::WThreads
        | FieldId::MnThreads
        | FieldId::SvThreads
        | FieldId::PfThreads
        | FieldId::DolThreads
        | FieldId::OmThreads
        | FieldId::CoThreads => vec![text(
            "How many CPU threads transcription uses. Automatic picks your core \
             count; lower it to leave room for other work.",
        )],
        FieldId::OvThreads => vec![text("OpenVINO ignores this for now.")],
        FieldId::WPrompt => vec![
            text(
                "Words and style Whisper should expect, such as names and terms \
                 you use: \"Voxtype, Hyprland, Claude.\"",
            ),
            dim("Enter types it."),
        ],
        FieldId::WFlashAttention => vec![text(
            "Faster on a graphics card, especially with large models. Turn it \
             off if Whisper hangs on an older driver.",
        )],
        FieldId::WOnDemandLoading
        | FieldId::PkOnDemandLoading
        | FieldId::MnOnDemandLoading
        | FieldId::SvOnDemandLoading
        | FieldId::PfOnDemandLoading
        | FieldId::DolOnDemandLoading
        | FieldId::OmOnDemandLoading
        | FieldId::CoOnDemandLoading
        | FieldId::OvOnDemandLoading => vec![text(
            "Loads the model when you start recording and frees the memory \
             after. Saves memory if you dictate now and then; the first words \
             take a moment longer.",
        )],
        FieldId::WGpuIsolation => vec![text(
            "Transcribes in a separate process that exits afterwards, so the \
             graphics card's memory is freed and a laptop's GPU can sleep. Adds \
             a fraction of a second.",
        )],
        FieldId::PkModelType => vec![text(
            "Automatic reads it from the model's files. Leave it unless a custom \
             model is detected wrongly.",
        )],
        FieldId::MnQuantized => vec![text(
            "A smaller version of the model, two to three times faster on a CPU \
             and slightly less accurate.",
        )],
        FieldId::SvLanguage => vec![text(
            "Automatic detects it each time; zh Mandarin, yue Cantonese, ja \
             Japanese, ko Korean, en English.",
        )],
        FieldId::SvUseItn => vec![text(
            "Adds punctuation and writes numbers and dates as digits: \
             \"twenty twenty-six\" becomes \"2026\".",
        )],
        FieldId::CoLanguage => vec![text(
            "Cohere doesn't detect the language, so pick the one you speak. It \
             knows ar, de, en, es, fr, hi, it, ja, ko, nl, pt, ru, tr and zh.",
        )],
        FieldId::OvDevice => openvino_device_guidance(&f.ov_device),
        FieldId::WRemoteEndpoint => vec![
            text("The server's address, such as https://api.openai.com/v1."),
            dim("Enter types it."),
        ],
        FieldId::WRemoteApiKey => vec![
            text(
                "The server's key, saved as plain text in the config file. To \
                 keep it out of the file, set VOXTYPE_WHISPER_API_KEY instead.",
            ),
            dim("Enter types it; it stays hidden."),
        ],
        FieldId::WRemoteModel => vec![
            text("The model name the server expects. Empty means whisper-1."),
            dim("Enter types it."),
        ],
    }
}

fn openvino_device_guidance(device: &str) -> Vec<Line<'static>> {
    let device = device.to_ascii_uppercase();
    let (summary, packages, check) = match device.as_str() {
        "NPU" => (
            "Intel's dedicated low-power AI accelerator.",
            "openvino openvino-intel-npu-plugin intel-npu-driver",
            "Reboot after installing the driver; then verify /dev/accel/accel* exists.",
        ),
        "GPU" => (
            "Intel integrated or discrete GPU.",
            "openvino openvino-intel-gpu-plugin level-zero-loader intel-compute-runtime",
            "Verify the device node with: ls /dev/dri/renderD*",
        ),
        "CPU" => (
            "Portable CPU inference; no accelerator driver is required.",
            "openvino",
            "This is the simplest fallback when no Intel accelerator is available.",
        ),
        _ => (
            "Let OpenVINO choose from the installed device plugins.",
            "openvino, plus the NPU and/or GPU packages shown for those devices",
            "Install every plugin and driver that AUTO should be allowed to use.",
        ),
    };

    vec![
        Line::from(summary),
        Line::from(""),
        Line::from(Span::styled(
            "Arch Linux packages:",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(format!("  sudo pacman -S {}", packages)),
        Line::from(check),
        Line::from(""),
        Line::from(Span::styled(
            "Required for every device:",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(
            "Install Intel's version-matched OpenVINO GenAI C/C++ SDK archive and set openvino_dir to its extracted root.",
        ),
        Line::from(Span::styled(
            "The pip wheel and openvino-genai-bin do not ship the required libopenvino_genai_c.so C API library.",
            theme::warn(),
        )),
    ]
}

/// Whether the chosen model is on disk, and what saving does about it.
pub fn model_note(engine: &str, current: &str) -> String {
    let installed = installed_models_for(engine).iter().any(|i| i == current);
    let mut note = if installed {
        "Downloaded.".to_string()
    } else {
        "Not downloaded yet; saving downloads it.".to_string()
    };
    if engine == "parakeet" && model::is_streaming_compatible_parakeet(current) {
        note.push_str(" Works with streaming output.");
    }
    if engine == "cohere" {
        note.push_str(" About 3 GB on disk.");
    }
    note
}

fn model_guidance(engine: &str, current: &str) -> Vec<Line<'static>> {
    let count = model_catalog(engine).len();
    vec![
        Line::from(model_note(engine, current)),
        Line::from(Span::styled(
            format!(
                "Larger models are more accurate and slower. ←→ goes through all {}.",
                count
            ),
            theme::muted(),
        )),
    ]
}

/// `local` reads as `Local`, `cli` as `CLI`.
fn capitalize_mode(mode: &str) -> String {
    match mode {
        "local" => "Local".to_string(),
        "remote" => "Remote".to_string(),
        "cli" => "CLI".to_string(),
        other => other.to_string(),
    }
}

/// `auto` reads as `Automatic`; language codes stay as they are written.
fn language_name(code: &str) -> String {
    if code == "auto" {
        "Automatic".to_string()
    } else {
        code.to_string()
    }
}

pub fn display_engine(engine: &str) -> &'static str {
    match engine {
        "whisper" => "Whisper",
        "parakeet" => "Parakeet",
        "moonshine" => "Moonshine",
        "sensevoice" => "SenseVoice",
        "paraformer" => "Paraformer",
        "dolphin" => "Dolphin",
        "omnilingual" => "Omnilingual",
        "cohere" => "Cohere",
        "openvino" => "OpenVINO",
        "soniox" => "Soniox",
        _ => "Engine",
    }
}

pub fn handle_key(app: &mut App, key: KeyEvent) -> Action {
    let state = match app.engine.as_mut() {
        Some(s) => s,
        None => return Action::None,
    };

    // While inline-editing a text field, route every key into the input until
    // the user commits or cancels. Esc / Ctrl-C cancel; Enter commits.
    if state.editing.is_some() {
        return handle_edit_key(state, key);
    }

    match key.code {
        KeyCode::Up | KeyCode::Char('k') => {
            state.move_cursor(-1);
            Action::None
        }
        KeyCode::Down | KeyCode::Char('j') => {
            state.move_cursor(1);
            Action::None
        }
        KeyCode::Left | KeyCode::Char('h') => {
            state.cycle(-1);
            Action::None
        }
        KeyCode::Right | KeyCode::Char('l') | KeyCode::Char(' ') => {
            state.cycle(1);
            Action::None
        }
        // `i` / Enter open inline-edit on text-editable fields.
        KeyCode::Enter | KeyCode::Char('i') => {
            state.start_edit_if_text_field();
            Action::None
        }
        KeyCode::Char('s') => state.save(),
        KeyCode::Char('r') => {
            state.reset();
            Action::None
        }
        _ => Action::None,
    }
}

fn handle_edit_key(state: &mut EngineState, key: KeyEvent) -> Action {
    let Some(editing) = state.editing.as_mut() else {
        return Action::None;
    };
    match editing.input.handle_key(key) {
        TextInputResult::Continue => Action::None,
        TextInputResult::Commit => {
            let buf = editing.input.buffer().to_string();
            let field = editing.field;
            state.editing = None;
            state.commit_text_edit(field, buf);
            Action::None
        }
        TextInputResult::Cancel => {
            state.editing = None;
            Action::None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line_text(lines: &[Line<'_>]) -> String {
        lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn openvino_engine_exposes_device_configuration() {
        let rows = rows_for_engine_with_mode("openvino", "local");
        assert!(rows.contains(&FieldId::OvModel));
        assert!(rows.contains(&FieldId::OvDevice));
        assert!(rows.contains(&FieldId::OvLanguage));
        assert!(rows.contains(&FieldId::OvOnDemandLoading));
    }

    #[test]
    fn openvino_tui_guidance_matches_selected_device() {
        let gpu = line_text(&openvino_device_guidance("GPU"));
        assert!(gpu.contains("openvino-intel-gpu-plugin"));
        assert!(gpu.contains("intel-compute-runtime"));
        assert!(!gpu.contains("intel-npu-driver"));

        let npu = line_text(&openvino_device_guidance("NPU"));
        assert!(npu.contains("openvino-intel-npu-plugin"));
        assert!(npu.contains("intel-npu-driver"));
        assert!(!npu.contains("intel-compute-runtime"));
    }
}
