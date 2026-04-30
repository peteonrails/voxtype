//! Engine section: per-engine tunables for the active transcription engine.
//!
//! The first row picks the engine itself; subsequent rows show the fields
//! that engine actually has. Switching engines on the form does not change
//! the value of fields you've edited for other engines — they're held in
//! memory until you save.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use super::app::{Action, App};
use super::common::{
    self, FeedbackLevel as CommonFeedback, FormRowSpec, TextInput, TextInputResult,
};
use super::config_editor::{ConfigEditor, EditorError};
use crate::setup::binary::{self, EngineFamily, InstallKind, Variant};
use crate::setup::model;

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
    pub binary_switch_blocked: Option<&'static str>,
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
}

/// Model catalogs per engine. Whisper/Parakeet/Moonshine/SenseVoice come from
/// the central setup::model registry; Paraformer/Dolphin/Omnilingual aren't
/// registered yet, so we hardcode their canonical defaults.
fn model_catalog(engine: &str) -> Vec<&'static str> {
    match engine {
        "whisper" => model::valid_model_names(),
        "parakeet" => model::valid_parakeet_model_names(),
        "moonshine" => model::valid_moonshine_model_names(),
        "sensevoice" => model::valid_sensevoice_model_names(),
        "paraformer" => vec!["paraformer-zh", "paraformer-en"],
        "dolphin" => vec!["dolphin-base"],
        "omnilingual" => vec!["omnilingual-300m"],
        "cohere" => vec!["cohere-transcribe-int8"],
        _ => Vec::new(),
    }
}

/// Default model name baked into voxtype for each ONNX engine. Used when we
/// have to materialize a fresh `[engine]` table because the user just made it
/// the active engine for the first time — those structs require `model` and
/// the validator rejects a partial table.
const fn default_model(engine: &str) -> &'static str {
    match engine.as_bytes() {
        b"parakeet" => "parakeet-tdt-0.6b-v3",
        b"moonshine" => "base",
        b"sensevoice" => "sensevoice-small",
        b"paraformer" => "paraformer-zh",
        b"dolphin" => "dolphin-base",
        b"omnilingual" => "omnilingual-300m",
        b"cohere" => "cohere-transcribe-int8",
        _ => "",
    }
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
}

const ENGINE_CHOICES: &[&str] = &[
    "whisper",
    "parakeet",
    "moonshine",
    "sensevoice",
    "paraformer",
    "dolphin",
    "omnilingual",
    "cohere",
];

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
                .unwrap_or_else(|| "auto".to_string()),
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
                .unwrap_or_else(|| "auto".to_string()),
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
            dol_on_demand_loading: ed
                .get_bool("dolphin", "on_demand_loading")
                .unwrap_or(false),
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
            co_on_demand_loading: ed
                .get_bool("cohere", "on_demand_loading")
                .unwrap_or(false),
            co_section_existed: ed.get_string("cohere", "model").is_some(),
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
        if inv.install_kind == InstallKind::Source {
            // Source builds can't be hot-swapped; whether the running binary
            // supports the chosen engine depends on its compiled features.
            // Best we can do is flag it.
            let supported = match self.engine.as_str() {
                "whisper" => true,
                "parakeet" => inv.compiled_features.iter().any(|f| *f == "parakeet"),
                _ => inv
                    .compiled_features
                    .iter()
                    .any(|f| *f == self.engine.as_str()),
            };
            if !supported {
                self.binary_switch_blocked = Some(
                    "Source build: rebuild voxtype with the corresponding \
                     Cargo feature for this engine.",
                );
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
                s.variant.family() == needed
                    && s.installed
                    && s.runs_on_this_cpu
                    && s.gpu_available
            });
            match fallback {
                Some(s) => self.pending_variant_switch = Some(s.variant),
                None => {
                    self.binary_switch_blocked = Some(
                        "No installed binary supports this engine on this \
                         hardware. Install the matching voxtype variant first.",
                    );
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
                    message: format!("load: {}", e),
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

        match ed.save() {
            Ok(()) => {
                self.dirty_since_load = false;
                let pending = self.pending_variant_switch.take();
                self.feedback = Some(Feedback {
                    level: FeedbackLevel::Ok,
                    message: match pending {
                        Some(v) => format!(
                            "Saved. Switching binary to {} (will prompt for sudo)…",
                            v.display()
                        ),
                        None => format!("Saved to {}", ed.path().display()),
                    },
                });
                if let Some(v) = pending {
                    return Action::SwitchVariant(v);
                }
            }
            Err(e) => {
                self.feedback = Some(Feedback {
                    level: FeedbackLevel::Err,
                    message: format!("save: {}", e),
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
                    message: "Reverted unsaved changes".to_string(),
                });
            }
            Err(e) => {
                self.feedback = Some(Feedback {
                    level: FeedbackLevel::Err,
                    message: format!("reload: {}", e),
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
                let idx = ENGINE_CHOICES
                    .iter()
                    .position(|c| *c == self.engine)
                    .map(|i| i as i32)
                    .unwrap_or(0);
                let n = (idx + delta).rem_euclid(ENGINE_CHOICES.len() as i32);
                self.engine = ENGINE_CHOICES[n as usize].to_string();
                // Clamp cursor into the new engine's row range; keep it at row 1
                // when present so the user lands on the first engine-specific
                // field.
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
                    input: TextInput::new(
                        f.w_initial_prompt.clone().unwrap_or_default(),
                    ),
                });
                return; // Don't mark dirty until commit.
            }
            FieldId::WFlashAttention => f.w_flash_attention = !f.w_flash_attention,
            FieldId::WOnDemandLoading => f.w_on_demand_loading = !f.w_on_demand_loading,
            FieldId::WGpuIsolation => f.w_gpu_isolation = !f.w_gpu_isolation,
            FieldId::WRemoteEndpoint
            | FieldId::WRemoteApiKey
            | FieldId::WRemoteModel => {
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
        }
        self.dirty_since_load = true;
        self.feedback = None;
    }
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

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let state = match &app.engine {
        Some(s) => s,
        None => {
            let block = Block::default().borders(Borders::ALL).title("Engine");
            let inner = block.inner(area);
            f.render_widget(block, area);
            f.render_widget(
                Paragraph::new("Failed to load config; check ~/.config/voxtype/config.toml.")
                    .wrap(Wrap { trim: true }),
                inner,
            );
            return;
        }
    };

    let rows: Vec<FormRowSpec> = state.rows()
        .iter()
        .enumerate()
        .map(|(i, fid)| {
            let (label, value) = field_label_value(state, *fid);
            FormRowSpec::new(i == state.cursor, label, value)
        })
        .collect();

    let feedback_pair = state.feedback.as_ref().map(|fb| {
        (
            match fb.level {
                FeedbackLevel::Ok => CommonFeedback::Ok,
                FeedbackLevel::Err => CommonFeedback::Err,
            },
            fb.message.as_str(),
        )
    });

    common::render_form_with_guidance(
        f,
        area,
        "Engine",
        state.dirty_since_load,
        feedback_pair,
        &rows,
        guidance(state),
    );
}

fn field_label_value(state: &EngineState, fid: FieldId) -> (&'static str, String) {
    let f = &state.fields;
    match fid {
        FieldId::Engine => ("Engine", state.engine.clone()),

        FieldId::WModel => ("Whisper · model", f.w_model.clone()),
        FieldId::WMode => ("Whisper · execution mode", f.w_mode.clone()),
        FieldId::WLanguage => ("Whisper · language", f.w_language.clone()),
        FieldId::WTranslate => ("Whisper · translate to English", yesno(f.w_translate)),
        FieldId::WThreads => (
            "Whisper · threads",
            f.w_threads
                .map(|n| n.to_string())
                .unwrap_or_else(|| "auto".to_string()),
        ),
        FieldId::WPrompt => (
            "Whisper · initial prompt",
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
                    .unwrap_or_else(|| "(none)".to_string()),
            },
        ),
        FieldId::WFlashAttention => ("Whisper · flash attention", yesno(f.w_flash_attention)),
        FieldId::WOnDemandLoading => ("Whisper · on-demand model load", yesno(f.w_on_demand_loading)),
        FieldId::WGpuIsolation => ("Whisper · GPU isolation", yesno(f.w_gpu_isolation)),
        FieldId::WRemoteEndpoint => (
            "Whisper · remote endpoint",
            match state.editing.as_ref() {
                Some(e) if e.field == FieldId::WRemoteEndpoint => e.input.caret_string(),
                _ => f
                    .w_remote_endpoint
                    .clone()
                    .unwrap_or_else(|| "(unset)".to_string()),
            },
        ),
        FieldId::WRemoteApiKey => (
            "Whisper · remote API key",
            match state.editing.as_ref() {
                Some(e) if e.field == FieldId::WRemoteApiKey => mask(&e.input.caret_string()),
                _ => match f.w_remote_api_key.as_deref() {
                    None | Some("") => "(unset)".to_string(),
                    Some(_) => "•••••• (set; press Enter to edit)".to_string(),
                },
            },
        ),
        FieldId::WRemoteModel => (
            "Whisper · remote model",
            match state.editing.as_ref() {
                Some(e) if e.field == FieldId::WRemoteModel => e.input.caret_string(),
                _ => f
                    .w_remote_model
                    .clone()
                    .unwrap_or_else(|| "(unset)".to_string()),
            },
        ),

        FieldId::PkModel => ("Parakeet · model", f.pk_model.clone()),
        FieldId::PkModelType => (
            "Parakeet · model architecture",
            f.pk_model_type
                .as_deref()
                .unwrap_or("auto-detect")
                .to_string(),
        ),
        FieldId::PkOnDemandLoading => {
            ("Parakeet · on-demand model load", yesno(f.pk_on_demand_loading))
        }

        FieldId::MnModel => ("Moonshine · model", f.mn_model.clone()),
        FieldId::MnQuantized => ("Moonshine · use quantized model", yesno(f.mn_quantized)),
        FieldId::MnThreads => (
            "Moonshine · threads",
            f.mn_threads
                .map(|n| n.to_string())
                .unwrap_or_else(|| "auto".to_string()),
        ),
        FieldId::MnOnDemandLoading => {
            ("Moonshine · on-demand model load", yesno(f.mn_on_demand_loading))
        }

        FieldId::SvModel => ("SenseVoice · model", f.sv_model.clone()),
        FieldId::SvLanguage => ("SenseVoice · language", f.sv_language.clone()),
        FieldId::SvUseItn => (
            "SenseVoice · inverse text normalization",
            yesno(f.sv_use_itn),
        ),
        FieldId::SvThreads => (
            "SenseVoice · threads",
            f.sv_threads
                .map(|n| n.to_string())
                .unwrap_or_else(|| "auto".to_string()),
        ),
        FieldId::SvOnDemandLoading => {
            ("SenseVoice · on-demand model load", yesno(f.sv_on_demand_loading))
        }

        FieldId::PfModel => ("Paraformer · model", f.pf_model.clone()),
        FieldId::PfThreads => (
            "Paraformer · threads",
            f.pf_threads
                .map(|n| n.to_string())
                .unwrap_or_else(|| "auto".to_string()),
        ),
        FieldId::PfOnDemandLoading => {
            ("Paraformer · on-demand model load", yesno(f.pf_on_demand_loading))
        }

        FieldId::DolModel => ("Dolphin · model", f.dol_model.clone()),
        FieldId::DolThreads => (
            "Dolphin · threads",
            f.dol_threads
                .map(|n| n.to_string())
                .unwrap_or_else(|| "auto".to_string()),
        ),
        FieldId::DolOnDemandLoading => {
            ("Dolphin · on-demand model load", yesno(f.dol_on_demand_loading))
        }

        FieldId::OmModel => ("Omnilingual · model", f.om_model.clone()),
        FieldId::OmThreads => (
            "Omnilingual · threads",
            f.om_threads
                .map(|n| n.to_string())
                .unwrap_or_else(|| "auto".to_string()),
        ),
        FieldId::OmOnDemandLoading => {
            ("Omnilingual · on-demand model load", yesno(f.om_on_demand_loading))
        }

        FieldId::CoModel => ("Cohere · model", f.co_model.clone()),
        FieldId::CoLanguage => ("Cohere · language", f.co_language.clone()),
        FieldId::CoThreads => (
            "Cohere · threads",
            f.co_threads
                .map(|n| n.to_string())
                .unwrap_or_else(|| "auto".to_string()),
        ),
        FieldId::CoOnDemandLoading => (
            "Cohere · on-demand model load",
            yesno(f.co_on_demand_loading),
        ),
    }
}

fn yesno(b: bool) -> String {
    (if b { "yes" } else { "no" }).to_string()
}

/// Mask a secret value for on-screen display while editing — show only the
/// final character so the user can verify they're typing what they intended,
/// but don't render the full key in the form row.
fn mask(s: &str) -> String {
    if s.is_empty() {
        return s.to_string();
    }
    let last = s.chars().last().map(|c| c.to_string()).unwrap_or_default();
    let bullets: String = std::iter::repeat('•').take(s.chars().count() - 1).collect();
    format!("{}{}", bullets, last)
}

fn heading(text: impl Into<String>) -> Line<'static> {
    Line::from(Span::styled(
        text.into(),
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    ))
}

fn engine_guidance(state: &EngineState) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();

    // Banner about a pending binary switch (or a blocked one) goes first so
    // the user sees it without scrolling.
    if let Some(target) = state.pending_variant_switch {
        lines.push(Line::from(Span::styled(
            "⚠ Binary switch required",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(format!(
            "This engine needs the {} family. Saving will also switch the binary to:",
            family_name(EngineState::required_family(&state.engine))
        )));
        lines.push(Line::from(Span::styled(
            format!("    {} ({})", target.display(), target.binary_name()),
            Style::default().fg(Color::Cyan),
        )));
        lines.push(Line::from(Span::styled(
            "    Press s to save; pkexec will prompt for sudo.",
            Style::default().fg(Color::Gray),
        )));
        lines.push(Line::from(""));
    } else if let Some(reason) = state.binary_switch_blocked {
        lines.push(Line::from(Span::styled(
            "⚠ Cannot switch binary",
            Style::default()
                .fg(Color::Red)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(reason.to_string()));
        lines.push(Line::from(""));
    }

    lines.push(heading("Active engine"));
    lines.push(Line::from(""));
    for (name, desc) in [
        (
            "whisper",
            "OpenAI Whisper via whisper.cpp. Default. Multilingual; best \
             general-purpose accuracy.",
        ),
        (
            "parakeet",
            "NVIDIA Parakeet TDT/CTC via ONNX Runtime. Tops the Open ASR \
             Leaderboard for English.",
        ),
        (
            "moonshine",
            "Useful Sensors Moonshine. Encoder-decoder, low-latency, small \
             footprint. Good for English dictation.",
        ),
        (
            "sensevoice",
            "Alibaba SenseVoice-Small. Strong on Chinese / Japanese / Korean \
             / Cantonese / English in one model.",
        ),
        (
            "paraformer / dolphin / omnilingual",
            "Specialized FunASR models. Paraformer focuses on Chinese, \
             Dolphin is dictation-tuned, Omnilingual covers 1600 languages.",
        ),
        (
            "cohere",
            "Cohere Transcribe (Cohere Labs). #1 on the Open ASR Leaderboard \
             for English (5.42 WER). 14 languages. ~3 GB on disk.",
        ),
    ] {
        lines.push(Line::from(Span::styled(
            format!("{}: ", name),
            Style::default().add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(desc.to_string()));
        lines.push(Line::from(""));
    }
    lines.push(Line::from(Span::styled(
        "Engine choice and binary family are linked. The TUI will swap \
         the binary for you when needed.",
        Style::default().fg(Color::Gray),
    )));
    lines
}

fn family_name(family: EngineFamily) -> &'static str {
    match family {
        EngineFamily::Whisper => "Whisper",
        EngineFamily::Onnx => "ONNX",
    }
}

fn guidance(state: &EngineState) -> Vec<Line<'_>> {
    let f = &state.fields;
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

        FieldId::WMode => vec![
            heading("Whisper · execution mode"),
            Line::from(""),
            Line::from(Span::styled(
                "local: ",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from("Transcribe in-process via whisper-rs. Default; offline."),
            Line::from(""),
            Line::from(Span::styled(
                "remote: ",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from(
                "Send audio to an OpenAI-compatible Whisper API. Set \
                 [whisper] remote_endpoint and remote_api_key first.",
            ),
            Line::from(""),
            Line::from(Span::styled(
                "cli: ",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from(
                "Shell out to a `whisper` CLI binary. Useful for testing \
                 custom builds without rebuilding voxtype.",
            ),
        ],
        FieldId::WLanguage => vec![
            heading("Whisper · language"),
            Line::from(""),
            Line::from(
                "Two-letter language code or `auto`. Auto-detect costs ~50ms \
                 on the first chunk; lock to a code when you only ever \
                 dictate in one language.",
            ),
            Line::from(""),
            Line::from(Span::styled(
                "Multi-language allowlists (e.g. \"en,fr,de\") can be set in \
                 config.toml as an array.",
                Style::default().fg(Color::Gray),
            )),
        ],
        FieldId::WTranslate => vec![
            heading("Whisper · translate to English"),
            Line::from(""),
            Line::from(
                "When on, Whisper translates non-English speech to English in \
                 the transcript.",
            ),
            Line::from(""),
            Line::from(
                "Useful for multilingual meetings where you want a single \
                 English transcript.",
            ),
        ],
        FieldId::WThreads => vec![
            heading("Whisper · threads"),
            Line::from(""),
            Line::from(
                "Number of CPU threads Whisper uses. `auto` lets voxtype \
                 pick (typically your physical-core count).",
            ),
            Line::from(""),
            Line::from(
                "Lower this to leave headroom for other work. Bump it for max \
                 throughput on a CPU-only setup.",
            ),
        ],
        FieldId::WPrompt => {
            let mut lines = vec![
                heading("Whisper · initial prompt"),
                Line::from(""),
                Line::from(
                    "Hints Whisper about terminology, capitalization, or formatting. \
                     Whisper biases its output toward what the prompt establishes.",
                ),
                Line::from(""),
                Line::from(
                    "Useful for proper nouns and technical terms. Examples: \
                     \"Voxtype, Hyprland, Claude.\" or \"Transcribe with proper \
                     capitalization and punctuation.\"",
                ),
                Line::from(""),
                Line::from(Span::styled(
                    "Press Enter or i to edit. While editing: type to insert, \
                     Backspace/Delete to remove, Enter commits, Esc cancels. \
                     Ctrl-W deletes the previous word; Ctrl-U clears the line.",
                    Style::default().fg(Color::Gray),
                )),
            ];
            if state.editing.as_ref().is_some_and(|e| e.field == FieldId::WPrompt) {
                lines.insert(
                    0,
                    Line::from(Span::styled(
                        "✎ Editing — Enter to commit, Esc to cancel",
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    )),
                );
                lines.insert(1, Line::from(""));
            }
            lines
        }
        FieldId::WFlashAttention => vec![
            heading("Whisper · flash attention"),
            Line::from(""),
            Line::from(
                "GPU optimization that reduces attention-layer memory \
                 bandwidth. Faster on capable cards, especially on large-v3.",
            ),
            Line::from(""),
            Line::from(
                "No effect on CPU runs. A few older driver combos crash with \
                 it on; turn it off if Whisper hangs.",
            ),
        ],
        FieldId::WOnDemandLoading => vec![
            heading("Whisper · on-demand model loading"),
            Line::from(""),
            Line::from(
                "Loads the model only when recording starts; unloads at idle. \
                 Frees ~1-2 GB of RAM between dictations.",
            ),
            Line::from(""),
            Line::from(
                "Adds a one-shot delay on the first key press of each \
                 dictation. Worth it for sporadic dictation; not worth it \
                 for constant use.",
            ),
        ],
        FieldId::WGpuIsolation => vec![
            heading("Whisper · GPU isolation"),
            Line::from(""),
            Line::from(
                "Each transcription runs in a short-lived subprocess that \
                 exits afterward, releasing all VRAM.",
            ),
            Line::from(""),
            Line::from(
                "Useful on hybrid-graphics laptops to let the discrete GPU \
                 power down between dictations. Adds ~100-300ms of subprocess \
                 startup per transcription.",
            ),
        ],

        FieldId::PkModelType => vec![
            heading("Parakeet · model architecture"),
            Line::from(""),
            Line::from(Span::styled(
                "auto-detect: ",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from(
                "voxtype inspects the model directory and picks tdt or ctc \
                 based on which ONNX files are present.",
            ),
            Line::from(""),
            Line::from(Span::styled(
                "tdt: ",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from(
                "Token-and-Duration Transducer. Recommended; what Parakeet's \
                 reference checkpoints use.",
            ),
            Line::from(""),
            Line::from(Span::styled(
                "ctc: ",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from(
                "CTC encoder-only models. Smaller, faster, slightly lower \
                 accuracy.",
            ),
            Line::from(""),
            Line::from(Span::styled(
                "Leave at auto-detect unless you have reason to override.",
                Style::default().fg(Color::Gray),
            )),
        ],
        FieldId::PkOnDemandLoading => vec![
            heading("Parakeet · on-demand model loading"),
            Line::from(""),
            Line::from(
                "Loads the Parakeet model only when recording starts; unloads \
                 at idle.",
            ),
            Line::from(""),
            Line::from(
                "Same trade-off as Whisper: frees memory between dictations \
                 at the cost of first-keystroke latency.",
            ),
        ],

        FieldId::MnQuantized => vec![
            heading("Moonshine · use quantized model"),
            Line::from(""),
            Line::from(
                "Moonshine ships int8-quantized weights alongside full \
                 precision. Quantized is ~2-3x faster on CPU at a small \
                 accuracy cost.",
            ),
            Line::from(""),
            Line::from(
                "Falls back to full precision if the quantized files aren't \
                 present in the model directory.",
            ),
        ],
        FieldId::MnThreads => threads_guidance("Moonshine"),
        FieldId::MnOnDemandLoading => on_demand_guidance("Moonshine"),

        FieldId::SvLanguage => vec![
            heading("SenseVoice · language"),
            Line::from(""),
            Line::from(
                "SenseVoice is multilingual across CJK + English. Pick a \
                 specific language to skip the language-detection step.",
            ),
            Line::from(""),
            Line::from(Span::styled(
                "auto: ",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from("Detect per-recording (default)."),
            Line::from(Span::styled(
                "zh / yue: ",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from("Mandarin / Cantonese."),
            Line::from(Span::styled(
                "ja / ko: ",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from("Japanese / Korean."),
            Line::from(Span::styled(
                "en: ",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from("English."),
        ],
        FieldId::SvUseItn => vec![
            heading("SenseVoice · inverse text normalization"),
            Line::from(""),
            Line::from(
                "Adds punctuation and converts spoken numbers/dates to their \
                 written form (\"twenty twenty-six\" → \"2026\").",
            ),
            Line::from(""),
            Line::from(
                "Recommended on. Turn off if you want raw token output for \
                 your own post-processing.",
            ),
        ],
        FieldId::SvThreads => threads_guidance("SenseVoice"),
        FieldId::SvOnDemandLoading => on_demand_guidance("SenseVoice"),

        FieldId::PfThreads => threads_guidance("Paraformer"),
        FieldId::PfOnDemandLoading => on_demand_guidance("Paraformer"),

        FieldId::DolThreads => threads_guidance("Dolphin"),
        FieldId::DolOnDemandLoading => on_demand_guidance("Dolphin"),

        FieldId::OmThreads => threads_guidance("Omnilingual"),
        FieldId::OmOnDemandLoading => on_demand_guidance("Omnilingual"),

        FieldId::CoLanguage => vec![
            heading("Cohere · language"),
            Line::from(""),
            Line::from(
                "Cohere Transcribe officially supports 14 languages. Pick the \
                 one you'll be dictating in; Cohere does not auto-detect.",
            ),
            Line::from(""),
            Line::from(Span::styled(
                "Supported codes:",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from(
                "  ar (Arabic), de (German), en (English), es (Spanish), \
                 fr (French), hi (Hindi), it (Italian), ja (Japanese), \
                 ko (Korean), nl (Dutch), pt (Portuguese), ru (Russian), \
                 tr (Turkish), zh (Chinese)",
            ),
        ],
        FieldId::CoThreads => threads_guidance("Cohere"),
        FieldId::CoOnDemandLoading => on_demand_guidance("Cohere"),

        FieldId::WRemoteEndpoint => vec![
            heading("Whisper · remote endpoint"),
            Line::from(""),
            Line::from(
                "OpenAI-compatible Whisper API base URL. voxtype POSTs audio \
                 multipart/form-data to <endpoint>/audio/transcriptions.",
            ),
            Line::from(""),
            Line::from(
                "Examples: https://api.openai.com/v1, http://localhost:9000/v1 \
                 (whisper.cpp server), https://api.groq.com/openai/v1.",
            ),
            Line::from(""),
            Line::from(Span::styled(
                "Press Enter to edit. Esc cancels.",
                Style::default().fg(Color::Gray),
            )),
        ],
        FieldId::WRemoteApiKey => vec![
            heading("Whisper · remote API key"),
            Line::from(""),
            Line::from(
                "Bearer token for the remote endpoint. Stored as plain text \
                 in config.toml — protect that file accordingly.",
            ),
            Line::from(""),
            Line::from(
                "If you'd rather not have it on disk, set the \
                 VOXTYPE_WHISPER_API_KEY environment variable instead and \
                 leave this unset.",
            ),
            Line::from(""),
            Line::from(Span::styled(
                "Press Enter to edit. Display is masked while not editing.",
                Style::default().fg(Color::Gray),
            )),
        ],
        FieldId::WRemoteModel => vec![
            heading("Whisper · remote model"),
            Line::from(""),
            Line::from(
                "Model name to send with each request (the `model` field in \
                 the multipart form). Defaults to whisper-1 if unset.",
            ),
            Line::from(""),
            Line::from(
                "Common values: whisper-1 (OpenAI), whisper-large-v3 \
                 (Groq, Together), whisper.cpp (whisper.cpp server). Check \
                 your provider's docs.",
            ),
        ],
    }
}

fn model_guidance(engine: &str, current: &str) -> Vec<Line<'static>> {
    let catalog = model_catalog(engine);
    let installed = installed_models_for(engine);
    let mut lines = vec![
        heading(format!("{} · model", display_engine(engine))),
        Line::from(""),
        Line::from(format!(
            "Inference checkpoint voxtype loads for {}. ←→ cycles through \
             the models voxtype knows about; pick whichever balances accuracy \
             and speed for your hardware.",
            display_engine(engine)
        )),
        Line::from(""),
    ];
    if !catalog.is_empty() {
        lines.push(Line::from(Span::styled(
            "Available  ( ● = installed,  · = not downloaded )",
            Style::default().add_modifier(Modifier::BOLD),
        )));
        for name in &catalog {
            let active = *name == current;
            let inst = installed.iter().any(|i| i == name);
            let cursor = if active { "▸ " } else { "  " };
            let marker = if inst { "●" } else { "·" };
            let suffix = if inst { "" } else { "  (not downloaded)" };
            let style = if !inst {
                Style::default().fg(Color::Gray)
            } else if active {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            lines.push(Line::from(Span::styled(
                format!("  {}{} {}{}", cursor, marker, name, suffix),
                style,
            )));
        }
        lines.push(Line::from(""));
    }
    lines.push(Line::from(Span::styled(
        "Models you haven't downloaded yet show up here too. Switch to one, \
         save, then run `voxtype setup model` to fetch the weights.",
        Style::default().fg(Color::Gray),
    )));
    if engine == "cohere" {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "⚠ Cohere's int8 model is ~3 GB on disk — heaviest of the bundled \
             engines. Make sure you've got the space before downloading.",
            Style::default().fg(Color::Yellow),
        )));
    }
    lines
}

/// Installed models on disk for a given engine. Mirrors the inventory the old
/// Models section used to show.
fn installed_models_for(engine: &str) -> Vec<String> {
    use crate::config::Config;
    let dir = Config::models_dir();
    let catalog = model_catalog(engine);
    catalog
        .into_iter()
        .filter(|name| {
            if engine == "whisper" {
                dir.join(format!("ggml-{}.bin", name)).exists()
            } else {
                let p = dir.join(name);
                p.exists() || p.is_dir()
            }
        })
        .map(|s| s.to_string())
        .collect()
}

fn display_engine(engine: &str) -> &'static str {
    match engine {
        "whisper" => "Whisper",
        "parakeet" => "Parakeet",
        "moonshine" => "Moonshine",
        "sensevoice" => "SenseVoice",
        "paraformer" => "Paraformer",
        "dolphin" => "Dolphin",
        "omnilingual" => "Omnilingual",
        "cohere" => "Cohere",
        _ => "Engine",
    }
}

fn threads_guidance(engine: &str) -> Vec<Line<'static>> {
    vec![
        heading(format!("{} · threads", engine)),
        Line::from(""),
        Line::from(format!(
            "Number of CPU threads ONNX Runtime uses for {} inference. \
             `auto` lets voxtype pick (typically physical-core count).",
            engine
        )),
        Line::from(""),
        Line::from(
            "Lower it to leave CPU headroom for other tasks. Bump to your \
             core count for max throughput.",
        ),
    ]
}

fn on_demand_guidance(engine: &str) -> Vec<Line<'static>> {
    vec![
        heading(format!("{} · on-demand model loading", engine)),
        Line::from(""),
        Line::from(format!(
            "Load the {} model only when recording starts; unload at idle.",
            engine
        )),
        Line::from(""),
        Line::from(
            "Frees memory between dictations at the cost of first-keystroke \
             latency. Worth it for sporadic dictation.",
        ),
    ]
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
            if state.start_edit_if_text_field() {
                Action::None
            } else {
                Action::None
            }
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
