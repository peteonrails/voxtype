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
use super::common::{self, FeedbackLevel as CommonFeedback, FormRowSpec};
use super::config_editor::{ConfigEditor, EditorError};

#[derive(Debug, Clone)]
pub struct EngineState {
    pub engine: String,
    pub fields: AllFields,
    pub cursor: usize,
    pub feedback: Option<Feedback>,
    pub dirty_since_load: bool,
}

#[derive(Debug, Clone, Default)]
pub struct AllFields {
    // Whisper
    pub w_mode: String,
    pub w_language: String,
    pub w_translate: bool,
    pub w_threads: Option<i64>,
    pub w_initial_prompt: Option<String>,
    pub w_flash_attention: bool,
    pub w_on_demand_loading: bool,
    pub w_gpu_isolation: bool,

    // Parakeet
    pub pk_model_type: Option<String>, // "tdt", "ctc", or None for auto-detect
    pub pk_on_demand_loading: bool,

    // Moonshine
    pub mn_quantized: bool,
    pub mn_threads: Option<i64>,
    pub mn_on_demand_loading: bool,

    // SenseVoice
    pub sv_language: String,
    pub sv_use_itn: bool,
    pub sv_threads: Option<i64>,
    pub sv_on_demand_loading: bool,

    // Paraformer
    pub pf_threads: Option<i64>,
    pub pf_on_demand_loading: bool,

    // Dolphin
    pub dol_threads: Option<i64>,
    pub dol_on_demand_loading: bool,

    // Omnilingual
    pub om_threads: Option<i64>,
    pub om_on_demand_loading: bool,
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
enum FieldId {
    Engine,

    // Whisper
    WMode,
    WLanguage,
    WTranslate,
    WThreads,
    WPrompt,
    WFlashAttention,
    WOnDemandLoading,
    WGpuIsolation,

    // Parakeet
    PkModelType,
    PkOnDemandLoading,

    // Moonshine
    MnQuantized,
    MnThreads,
    MnOnDemandLoading,

    // SenseVoice
    SvLanguage,
    SvUseItn,
    SvThreads,
    SvOnDemandLoading,

    // Paraformer
    PfThreads,
    PfOnDemandLoading,

    // Dolphin
    DolThreads,
    DolOnDemandLoading,

    // Omnilingual
    OmThreads,
    OmOnDemandLoading,
}

const ENGINE_CHOICES: &[&str] = &[
    "whisper",
    "parakeet",
    "moonshine",
    "sensevoice",
    "paraformer",
    "dolphin",
    "omnilingual",
];

const MODE_CHOICES: &[&str] = &["local", "remote", "cli"];
const LANG_CHOICES: &[&str] = &[
    "auto", "en", "fr", "de", "it", "es", "pt", "nl", "pl", "zh", "ja", "ko", "ru", "ar",
];
const SV_LANG_CHOICES: &[&str] = &["auto", "zh", "en", "ja", "ko", "yue"];
const PARAKEET_MODEL_TYPES: &[Option<&str>] = &[None, Some("tdt"), Some("ctc")];

fn rows_for_engine(engine: &str) -> Vec<FieldId> {
    let mut rows = vec![FieldId::Engine];
    match engine {
        "whisper" => rows.extend_from_slice(&[
            FieldId::WMode,
            FieldId::WLanguage,
            FieldId::WTranslate,
            FieldId::WThreads,
            FieldId::WPrompt,
            FieldId::WFlashAttention,
            FieldId::WOnDemandLoading,
            FieldId::WGpuIsolation,
        ]),
        "parakeet" => rows.extend_from_slice(&[FieldId::PkModelType, FieldId::PkOnDemandLoading]),
        "moonshine" => rows.extend_from_slice(&[
            FieldId::MnQuantized,
            FieldId::MnThreads,
            FieldId::MnOnDemandLoading,
        ]),
        "sensevoice" => rows.extend_from_slice(&[
            FieldId::SvLanguage,
            FieldId::SvUseItn,
            FieldId::SvThreads,
            FieldId::SvOnDemandLoading,
        ]),
        "paraformer" => {
            rows.extend_from_slice(&[FieldId::PfThreads, FieldId::PfOnDemandLoading])
        }
        "dolphin" => {
            rows.extend_from_slice(&[FieldId::DolThreads, FieldId::DolOnDemandLoading])
        }
        "omnilingual" => {
            rows.extend_from_slice(&[FieldId::OmThreads, FieldId::OmOnDemandLoading])
        }
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

            // Parakeet
            pk_model_type: ed.get_string("parakeet", "model_type"),
            pk_on_demand_loading: ed
                .get_bool("parakeet", "on_demand_loading")
                .unwrap_or(false),

            // Moonshine
            mn_quantized: ed.get_bool("moonshine", "quantized").unwrap_or(true),
            mn_threads: ed.get_int("moonshine", "threads"),
            mn_on_demand_loading: ed
                .get_bool("moonshine", "on_demand_loading")
                .unwrap_or(false),

            // SenseVoice
            sv_language: ed
                .get_string("sensevoice", "language")
                .unwrap_or_else(|| "auto".to_string()),
            sv_use_itn: ed.get_bool("sensevoice", "use_itn").unwrap_or(true),
            sv_threads: ed.get_int("sensevoice", "threads"),
            sv_on_demand_loading: ed
                .get_bool("sensevoice", "on_demand_loading")
                .unwrap_or(false),

            // Paraformer
            pf_threads: ed.get_int("paraformer", "threads"),
            pf_on_demand_loading: ed
                .get_bool("paraformer", "on_demand_loading")
                .unwrap_or(false),

            // Dolphin
            dol_threads: ed.get_int("dolphin", "threads"),
            dol_on_demand_loading: ed
                .get_bool("dolphin", "on_demand_loading")
                .unwrap_or(false),

            // Omnilingual
            om_threads: ed.get_int("omnilingual", "threads"),
            om_on_demand_loading: ed
                .get_bool("omnilingual", "on_demand_loading")
                .unwrap_or(false),
        };
        Ok(Self {
            engine,
            fields,
            cursor: 0,
            feedback: None,
            dirty_since_load: false,
        })
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

        // Whisper
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

        // Parakeet
        match &f.pk_model_type {
            Some(m) => ed.set_string("parakeet", "model_type", m),
            None => ed.unset("parakeet", "model_type"),
        }
        ed.set_bool("parakeet", "on_demand_loading", f.pk_on_demand_loading);

        // Moonshine
        ed.set_bool("moonshine", "quantized", f.mn_quantized);
        match f.mn_threads {
            Some(n) => ed.set_int("moonshine", "threads", n),
            None => ed.unset("moonshine", "threads"),
        }
        ed.set_bool("moonshine", "on_demand_loading", f.mn_on_demand_loading);

        // SenseVoice
        ed.set_string("sensevoice", "language", &f.sv_language);
        ed.set_bool("sensevoice", "use_itn", f.sv_use_itn);
        match f.sv_threads {
            Some(n) => ed.set_int("sensevoice", "threads", n),
            None => ed.unset("sensevoice", "threads"),
        }
        ed.set_bool("sensevoice", "on_demand_loading", f.sv_on_demand_loading);

        // Paraformer
        match f.pf_threads {
            Some(n) => ed.set_int("paraformer", "threads", n),
            None => ed.unset("paraformer", "threads"),
        }
        ed.set_bool("paraformer", "on_demand_loading", f.pf_on_demand_loading);

        // Dolphin
        match f.dol_threads {
            Some(n) => ed.set_int("dolphin", "threads", n),
            None => ed.unset("dolphin", "threads"),
        }
        ed.set_bool("dolphin", "on_demand_loading", f.dol_on_demand_loading);

        // Omnilingual
        match f.om_threads {
            Some(n) => ed.set_int("omnilingual", "threads", n),
            None => ed.unset("omnilingual", "threads"),
        }
        ed.set_bool("omnilingual", "on_demand_loading", f.om_on_demand_loading);

        match ed.save() {
            Ok(()) => {
                self.dirty_since_load = false;
                self.feedback = Some(Feedback {
                    level: FeedbackLevel::Ok,
                    message: format!("Saved to {}", ed.path().display()),
                });
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
                let max = rows_for_engine(&self.engine).len().saturating_sub(1);
                self.cursor = cursor.min(max);
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
        let len = rows_for_engine(&self.engine).len() as i32;
        if len == 0 {
            return;
        }
        let new = (self.cursor as i32 + delta).rem_euclid(len);
        self.cursor = new as usize;
    }

    fn current_field(&self) -> FieldId {
        let rows = rows_for_engine(&self.engine);
        rows.get(self.cursor).copied().unwrap_or(FieldId::Engine)
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
                let max = rows_for_engine(&self.engine).len().saturating_sub(1);
                self.cursor = self.cursor.min(max);
            }
            FieldId::WMode => f.w_mode = cycle_str(MODE_CHOICES, &f.w_mode, delta),
            FieldId::WLanguage => f.w_language = cycle_str(LANG_CHOICES, &f.w_language, delta),
            FieldId::WTranslate => f.w_translate = !f.w_translate,
            FieldId::WThreads => f.w_threads = cycle_threads(f.w_threads, delta),
            FieldId::WPrompt => {
                f.w_initial_prompt = match f.w_initial_prompt.take() {
                    Some(_) => None,
                    None => Some(
                        "Transcribe with proper capitalization and punctuation.".to_string(),
                    ),
                }
            }
            FieldId::WFlashAttention => f.w_flash_attention = !f.w_flash_attention,
            FieldId::WOnDemandLoading => f.w_on_demand_loading = !f.w_on_demand_loading,
            FieldId::WGpuIsolation => f.w_gpu_isolation = !f.w_gpu_isolation,

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

            FieldId::MnQuantized => f.mn_quantized = !f.mn_quantized,
            FieldId::MnThreads => f.mn_threads = cycle_threads(f.mn_threads, delta),
            FieldId::MnOnDemandLoading => f.mn_on_demand_loading = !f.mn_on_demand_loading,

            FieldId::SvLanguage => {
                f.sv_language = cycle_str(SV_LANG_CHOICES, &f.sv_language, delta)
            }
            FieldId::SvUseItn => f.sv_use_itn = !f.sv_use_itn,
            FieldId::SvThreads => f.sv_threads = cycle_threads(f.sv_threads, delta),
            FieldId::SvOnDemandLoading => f.sv_on_demand_loading = !f.sv_on_demand_loading,

            FieldId::PfThreads => f.pf_threads = cycle_threads(f.pf_threads, delta),
            FieldId::PfOnDemandLoading => f.pf_on_demand_loading = !f.pf_on_demand_loading,

            FieldId::DolThreads => f.dol_threads = cycle_threads(f.dol_threads, delta),
            FieldId::DolOnDemandLoading => f.dol_on_demand_loading = !f.dol_on_demand_loading,

            FieldId::OmThreads => f.om_threads = cycle_threads(f.om_threads, delta),
            FieldId::OmOnDemandLoading => f.om_on_demand_loading = !f.om_on_demand_loading,
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

    let rows: Vec<FormRowSpec> = rows_for_engine(&state.engine)
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
            f.w_initial_prompt
                .as_deref()
                .map(|s| {
                    if s.len() > 30 {
                        format!("{}…", &s[..30])
                    } else {
                        s.to_string()
                    }
                })
                .unwrap_or_else(|| "(none)".to_string()),
        ),
        FieldId::WFlashAttention => ("Whisper · flash attention", yesno(f.w_flash_attention)),
        FieldId::WOnDemandLoading => ("Whisper · on-demand model load", yesno(f.w_on_demand_loading)),
        FieldId::WGpuIsolation => ("Whisper · GPU isolation", yesno(f.w_gpu_isolation)),

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

        FieldId::PfThreads => (
            "Paraformer · threads",
            f.pf_threads
                .map(|n| n.to_string())
                .unwrap_or_else(|| "auto".to_string()),
        ),
        FieldId::PfOnDemandLoading => {
            ("Paraformer · on-demand model load", yesno(f.pf_on_demand_loading))
        }

        FieldId::DolThreads => (
            "Dolphin · threads",
            f.dol_threads
                .map(|n| n.to_string())
                .unwrap_or_else(|| "auto".to_string()),
        ),
        FieldId::DolOnDemandLoading => {
            ("Dolphin · on-demand model load", yesno(f.dol_on_demand_loading))
        }

        FieldId::OmThreads => (
            "Omnilingual · threads",
            f.om_threads
                .map(|n| n.to_string())
                .unwrap_or_else(|| "auto".to_string()),
        ),
        FieldId::OmOnDemandLoading => {
            ("Omnilingual · on-demand model load", yesno(f.om_on_demand_loading))
        }
    }
}

fn yesno(b: bool) -> String {
    (if b { "yes" } else { "no" }).to_string()
}

fn heading(text: impl Into<String>) -> Line<'static> {
    Line::from(Span::styled(
        text.into(),
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    ))
}

fn guidance(state: &EngineState) -> Vec<Line<'_>> {
    match state.current_field() {
        FieldId::Engine => vec![
            heading("Active engine"),
            Line::from(""),
            Line::from(Span::styled(
                "whisper: ",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from(
                "OpenAI Whisper via whisper.cpp. Default. Multilingual; \
                 best general-purpose accuracy.",
            ),
            Line::from(""),
            Line::from(Span::styled(
                "parakeet: ",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from(
                "NVIDIA Parakeet TDT/CTC via ONNX Runtime. Tops the Open ASR \
                 Leaderboard for English. Requires the parakeet feature.",
            ),
            Line::from(""),
            Line::from(Span::styled(
                "moonshine: ",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from(
                "Useful Sensors Moonshine. Encoder-decoder, low-latency, \
                 small footprint. Good for English dictation.",
            ),
            Line::from(""),
            Line::from(Span::styled(
                "sensevoice: ",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from(
                "Alibaba SenseVoice-Small. Strong on Chinese / Japanese / \
                 Korean / Cantonese / English in one model.",
            ),
            Line::from(""),
            Line::from(Span::styled(
                "paraformer / dolphin / omnilingual: ",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from(
                "Specialized FunASR models. Paraformer focuses on Chinese, \
                 Dolphin is dictation-tuned, Omnilingual covers 1600 languages.",
            ),
            Line::from(""),
            Line::from(Span::styled(
                "Switching engine here also requires an installed binary that \
                 supports it (see General → Variant).",
                Style::default().fg(Color::Gray),
            )),
        ],

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
        FieldId::WPrompt => vec![
            heading("Whisper · initial prompt"),
            Line::from(""),
            Line::from(
                "Hints Whisper about terminology, capitalization, or formatting. \
                 Whisper biases its output toward what the prompt establishes.",
            ),
            Line::from(""),
            Line::from(Span::styled(
                "TUI cycles between (none) and a sample. Edit the body in \
                 [whisper] initial_prompt directly for a custom prompt.",
                Style::default().fg(Color::Gray),
            )),
        ],
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
        KeyCode::Char('s') => state.save(),
        KeyCode::Char('r') => {
            state.reset();
            Action::None
        }
        _ => Action::None,
    }
}
