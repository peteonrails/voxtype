//! Shared sliding-window streaming settings (`[streaming]` in config.toml)
//! plus the per-engine on/off switch.
//!
//! The tuning knobs are consumed by `transcribe::sliding_window`'s streaming
//! engine — the same seven fields regardless of which backend wraps itself
//! in it (whisper.cpp local mode, OpenVINO GenAI). See
//! `config::streaming::StreamingConfig` for the authoritative field
//! documentation and `StreamingConfig::resolve` for how this section
//! relates to the older, deprecated per-engine `streaming_*` fields
//! (`[whisper] streaming_interval_secs`, `[openvino] streaming_interval_secs`,
//! …) — once `[streaming]` exists in config.toml at all, it wins outright
//! over those regardless of which fields it sets, so this section always
//! writes every field rather than only the ones the user touched.
//!
//! The on/off toggle is separate: it writes the active engine's own
//! `streaming: bool` (`[whisper]`, `[openvino]`, `[parakeet]`, `[soniox]`),
//! re-read at save time so a mid-session engine switch targets the right
//! table. Engines without a `streaming` field get a read-only toggle.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use super::app::{Action, App};
use super::common::{self, FeedbackLevel, FormRowSpec};
use super::config_editor::{ConfigEditor, EditorError};

#[derive(Debug, Clone)]
pub struct StreamingState {
    /// On/off for the active engine's streaming output. Mirrors the
    /// per-engine `streaming: bool` (`[whisper]`, `[openvino]`, `[parakeet]`,
    /// `[soniox]`) — deliberately NOT a `[streaming]` key, because the shared
    /// section is engine-agnostic tuning and can't own a per-engine switch.
    pub enabled: bool,
    /// The active engine name (`engine = "..."` in config.toml), read at load
    /// time and re-read at save time so a mid-session engine switch in the
    /// Engine section doesn't write the toggle to the wrong table.
    pub engine: String,
    /// Whether the active engine has a `streaming` field at all. Engines
    /// without one (cohere, moonshine, ...) can't stream; the toggle is
    /// read-only for them.
    pub streaming_supported: bool,
    pub interval_secs: f32,
    pub max_buffer_secs: f32,
    pub min_speech_rms: f32,
    pub min_audio_secs: f32,
    pub partial_min_words: i64,
    pub type_partials: bool,
    pub revision_mode: bool,
    /// True if `[streaming]` had at least one key set at load time. Mirrors
    /// the `*_section_existed` pattern in `engine.rs` for optional
    /// per-engine tables — kept so a user who never touches this section
    /// doesn't get a `[streaming]` table materialized in their config.toml
    /// purely from opening the TUI page, only from actually saving it.
    pub section_existed: bool,
    pub field: Field,
    pub feedback: Option<(FeedbackLevel, String)>,
    pub dirty_since_load: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Enabled,
    IntervalSecs,
    MaxBufferSecs,
    MinSpeechRms,
    MinAudioSecs,
    PartialMinWords,
    TypePartials,
    RevisionMode,
}
impl Field {
    const ALL: &'static [Field] = &[
        Field::Enabled,
        Field::IntervalSecs,
        Field::MaxBufferSecs,
        Field::MinSpeechRms,
        Field::MinAudioSecs,
        Field::PartialMinWords,
        Field::TypePartials,
        Field::RevisionMode,
    ];
}

impl StreamingState {
    pub fn load() -> Result<Self, EditorError> {
        let ed = ConfigEditor::load()?;
        let section_existed = ed.get_toml_value("streaming", "interval_secs").is_some()
            || ed.get_toml_value("streaming", "max_buffer_secs").is_some()
            || ed.get_toml_value("streaming", "min_speech_rms").is_some()
            || ed.get_toml_value("streaming", "min_audio_secs").is_some()
            || ed
                .get_toml_value("streaming", "partial_min_words")
                .is_some()
            || ed.get_toml_value("streaming", "type_partials").is_some()
            || ed.get_toml_value("streaming", "revision_mode").is_some();

        let engine = ed
            .get_string("", "engine")
            .unwrap_or_else(|| "whisper".to_string());
        let streaming_supported = matches!(
            engine.as_str(),
            "whisper" | "openvino" | "parakeet" | "soniox"
        );
        let enabled = if streaming_supported {
            ed.get_bool(&engine, "streaming").unwrap_or(false)
        } else {
            false
        };

        Ok(Self {
            enabled,
            engine,
            streaming_supported,
            interval_secs: ed.get_f32_or("streaming", "interval_secs", 0.8),
            max_buffer_secs: ed.get_f32_or("streaming", "max_buffer_secs", 29.0),
            min_speech_rms: ed.get_f32_or("streaming", "min_speech_rms", 0.005),
            min_audio_secs: ed.get_f32_or("streaming", "min_audio_secs", 1.0),
            partial_min_words: ed.get_int("streaming", "partial_min_words").unwrap_or(1),
            type_partials: ed.get_bool("streaming", "type_partials").unwrap_or(true),
            revision_mode: ed.get_bool("streaming", "revision_mode").unwrap_or(true),
            section_existed,
            field: Field::Enabled,
            feedback: None,
            dirty_since_load: false,
        })
    }

    /// Re-read the active engine so a switch made in the Engine section is
    /// reflected here. Called each time the section is opened; the state is
    /// otherwise cached for the session. The toggle value is only refreshed
    /// when the user hasn't edited it, so an unsaved change isn't clobbered
    /// by navigating away and back.
    pub fn refresh_engine(&mut self) {
        let ed = match ConfigEditor::load() {
            Ok(e) => e,
            Err(_) => return,
        };
        let engine = ed
            .get_string("", "engine")
            .unwrap_or_else(|| self.engine.clone());
        let streaming_supported = matches!(
            engine.as_str(),
            "whisper" | "openvino" | "parakeet" | "soniox"
        );
        if engine != self.engine {
            self.engine = engine;
            self.streaming_supported = streaming_supported;
            if !self.dirty_since_load {
                self.enabled = if streaming_supported {
                    ed.get_bool(&self.engine, "streaming").unwrap_or(false)
                } else {
                    false
                };
            }
        }
    }

    pub fn save(&mut self) -> Action {
        let mut ed = match ConfigEditor::load() {
            Ok(e) => e,
            Err(e) => {
                self.feedback = Some((FeedbackLevel::Err, format!("load: {}", e)));
                return Action::None;
            }
        };
        // Re-read the active engine so a mid-session switch in the Engine
        // section can't write the toggle to the table of the engine the user
        // was on when this section was first opened.
        let engine = ed
            .get_string("", "engine")
            .unwrap_or_else(|| self.engine.clone());
        let streaming_supported = matches!(
            engine.as_str(),
            "whisper" | "openvino" | "parakeet" | "soniox"
        );
        self.engine = engine;
        self.streaming_supported = streaming_supported;
        if streaming_supported {
            ed.set_bool(&self.engine, "streaming", self.enabled);
        }
        ed.set_f32("streaming", "interval_secs", self.interval_secs);
        ed.set_f32("streaming", "max_buffer_secs", self.max_buffer_secs);
        ed.set_f32("streaming", "min_speech_rms", self.min_speech_rms);
        ed.set_f32("streaming", "min_audio_secs", self.min_audio_secs);
        ed.set_int("streaming", "partial_min_words", self.partial_min_words);
        ed.set_bool("streaming", "type_partials", self.type_partials);
        ed.set_bool("streaming", "revision_mode", self.revision_mode);
        match ed.save() {
            Ok(()) => {
                self.section_existed = true;
                self.dirty_since_load = false;
                self.feedback = Some((
                    FeedbackLevel::Ok,
                    format!("Saved to {}", ed.path().display()),
                ));
            }
            Err(e) => self.feedback = Some((FeedbackLevel::Err, format!("save: {}", e))),
        }
        Action::None
    }

    pub fn reset(&mut self) {
        match Self::load() {
            Ok(fresh) => {
                let field = self.field;
                *self = fresh;
                self.field = field;
                self.feedback = Some((FeedbackLevel::Ok, "Reverted".to_string()));
            }
            Err(e) => self.feedback = Some((FeedbackLevel::Err, format!("reload: {}", e))),
        }
    }

    fn move_field(&mut self, delta: i32) {
        let len = Field::ALL.len() as i32;
        let cur = Field::ALL
            .iter()
            .position(|f| *f == self.field)
            .unwrap_or(0) as i32;
        let new = (cur + delta).rem_euclid(len);
        self.field = Field::ALL[new as usize];
    }

    fn cycle(&mut self, delta: i32) {
        match self.field {
            Field::Enabled => {
                // Read-only for engines without a streaming field; the
                // guidance panel explains why.
                if self.streaming_supported {
                    self.enabled = !self.enabled;
                }
            }
            Field::IntervalSecs => {
                self.interval_secs = (self.interval_secs + delta as f32 * 0.1).clamp(0.1, 5.0);
            }
            Field::MaxBufferSecs => {
                self.max_buffer_secs =
                    (self.max_buffer_secs + delta as f32 * 1.0).clamp(3.0, 60.0);
            }
            Field::MinSpeechRms => {
                self.min_speech_rms = (self.min_speech_rms + delta as f32 * 0.001).clamp(0.0, 0.1);
            }
            Field::MinAudioSecs => {
                self.min_audio_secs = (self.min_audio_secs + delta as f32 * 0.1).clamp(0.1, 5.0);
            }
            Field::PartialMinWords => {
                self.partial_min_words = (self.partial_min_words + delta as i64).clamp(1, 10);
            }
            Field::TypePartials => self.type_partials = !self.type_partials,
            Field::RevisionMode => self.revision_mode = !self.revision_mode,
        }
        self.dirty_since_load = true;
        self.feedback = None;
    }
}

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let state = match &app.streaming {
        Some(s) => s,
        None => {
            let block = Block::default().borders(Borders::ALL).title("Streaming");
            let inner = block.inner(area);
            f.render_widget(block, area);
            f.render_widget(
                Paragraph::new("Failed to load config.").wrap(Wrap { trim: true }),
                inner,
            );
            return;
        }
    };

    let rows = vec![
        FormRowSpec::new(
            state.field == Field::Enabled,
            "Streaming enabled",
            yesno(state.enabled),
        ),
        FormRowSpec::new(
            state.field == Field::IntervalSecs,
            "Tick interval",
            format!("{:.1}s", state.interval_secs),
        ),
        FormRowSpec::new(
            state.field == Field::MaxBufferSecs,
            "Max buffer",
            format!("{:.0}s", state.max_buffer_secs),
        ),
        FormRowSpec::new(
            state.field == Field::MinAudioSecs,
            "Min audio before first partial",
            format!("{:.1}s", state.min_audio_secs),
        ),
        FormRowSpec::new(
            state.field == Field::MinSpeechRms,
            "Min speech RMS",
            format!("{:.3}", state.min_speech_rms),
        ),
        FormRowSpec::new(
            state.field == Field::PartialMinWords,
            "Min stable words to commit",
            state.partial_min_words.to_string(),
        ),
        FormRowSpec::new(
            state.field == Field::TypePartials,
            "Type live at cursor",
            yesno(state.type_partials),
        ),
        FormRowSpec::new(
            state.field == Field::RevisionMode,
            "Revision mode",
            yesno(state.revision_mode),
        ),
    ];

    let feedback_pair = state
        .feedback
        .as_ref()
        .map(|(lvl, msg)| (*lvl, msg.as_str()));

    common::render_form_with_guidance(
        f,
        area,
        "Streaming",
        state.dirty_since_load,
        feedback_pair,
        &rows,
        guidance_for_field(state),
    );
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

fn guidance_for_field(state: &StreamingState) -> Vec<Line<'_>> {
    let mut lines = match state.field {
        Field::Enabled => {
            let mut lines = vec![
                heading("Streaming enabled"),
                Line::from(""),
            ];
            if state.streaming_supported {
                lines.push(Line::from(format!(
                    "Turns live partial transcription on/off for the active \
                     engine ({}) by writing [{}] streaming = true/false.",
                    state.engine, state.engine
                )));
                lines.push(Line::from(""));
                lines.push(Line::from(
                    "The knobs below tune the shared sliding-window engine \
                     that streams partials while you speak.",
                ));
            } else {
                lines.push(Line::from(Span::styled(
                    format!(
                        "The active engine ({}) does not support streaming \
                         output.",
                        state.engine
                    ),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )));
                lines.push(Line::from(""));
                lines.push(Line::from(
                    "Switch the engine to whisper, openvino, parakeet, or \
                     soniox to use live streaming.",
                ));
            }
            lines
        }
        Field::IntervalSecs => vec![
            heading("Tick interval"),
            Line::from(""),
            Line::from(
                "How often (seconds) the streaming engine re-transcribes \
                 the whole rolling buffer and looks for newly-stable text \
                 to commit.",
            ),
            Line::from(""),
            Line::from(
                "Lower = more responsive live text, higher inference load. \
                 If inference on your hardware/model regularly takes longer \
                 than this interval, ticks start falling behind — voxtype \
                 logs a warning (\"inference exceeded the tick interval\") \
                 when that happens. 0.8s is a reasonable default for \
                 NPU/GPU-accelerated models.",
            ),
        ],
        Field::MaxBufferSecs => vec![
            heading("Max buffer"),
            Line::from(""),
            Line::from(
                "Hard cap on how much audio the rolling buffer holds before \
                 old audio is dropped and the engine switches from \
                 \"growing\" to \"sliding\" diff mode.",
            ),
            Line::from(""),
            Line::from(
                "Per-tick re-transcription cost scales with buffer length, \
                 so a long session's first pass through this many seconds \
                 gets progressively more expensive until the cap is hit. \
                 Lowering this makes the transition to sliding mode happen \
                 sooner — sliding mode's own diffing can be less stable \
                 (Whisper occasionally re-words the buffer's leading edge \
                 slightly differently pass to pass), so lower isn't always \
                 better in practice. 29s is the current default.",
            ),
        ],
        Field::MinAudioSecs => vec![
            heading("Min audio before first partial"),
            Line::from(""),
            Line::from(
                "Seconds of buffered audio required before the very first \
                 transcription attempt of a session. Below this, ticks are \
                 skipped entirely (nothing to transcribe yet).",
            ),
        ],
        Field::MinSpeechRms => vec![
            heading("Min speech RMS"),
            Line::from(""),
            Line::from(
                "Whole-buffer RMS energy threshold below which a tick is \
                 treated as silence and skipped, preventing Whisper from \
                 hallucinating text on quiet/empty audio.",
            ),
            Line::from(""),
            Line::from(
                "0.005 is the ported nova-npu default. Raise it if you get \
                 phantom text during pauses; lower it if quiet speech is \
                 being skipped.",
            ),
        ],
        Field::PartialMinWords => vec![
            heading("Min stable words to commit"),
            Line::from(""),
            Line::from(
                "The conservative (non-revision) commit gate requires at \
                 least this many new words to agree across two consecutive \
                 ticks before typing them. Higher values commit less often \
                 but in bigger, more confident chunks.",
            ),
            Line::from(""),
            Line::from("Has no effect while Revision mode is on."),
        ],
        Field::TypePartials => vec![
            heading("Type live at cursor"),
            Line::from(""),
            Line::from(
                "When on, each committed delta is typed immediately at the \
                 cursor as it's confirmed. When off, text is only \
                 committed internally (as \"Final\" segments) rather than \
                 typed live — useful for file-output sessions that just \
                 want the finished transcript, not live keystrokes.",
            ),
        ],
        Field::RevisionMode => vec![
            heading("Revision mode (type-then-correct)"),
            Line::from(""),
            Line::from(
                "Types the current best-guess tail immediately instead of \
                 waiting for it to stabilize, correcting it later via \
                 backspace + retype if a following tick disagrees. On by \
                 default — the conservative gate below is the opt-in.",
            ),
            Line::from(""),
            Line::from(Span::styled(
                "Trade-off: ",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from(
                "more responsive live text, at the cost of visible \
                 flicker (type, then backspace, then retype) when Whisper \
                 changes its mind about a word — and a bookkeeping mistake \
                 here can in principle delete characters that were never \
                 voxtype's to begin with. Turn it off to fall back to the \
                 conservative gate, which never types something wrong but \
                 lags by a couple of ticks.",
            ),
        ],
    };

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!(
            "[streaming] {} in config.toml.",
            if state.section_existed {
                "section present"
            } else {
                "section not yet created — will be added on save"
            }
        ),
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(Span::styled(
        format!(
            "On/off toggle writes [{}] streaming (active engine).",
            state.engine
        ),
        Style::default().fg(Color::DarkGray),
    )));
    lines
}

pub fn handle_key(app: &mut App, key: KeyEvent) -> Action {
    let state = match app.streaming.as_mut() {
        Some(s) => s,
        None => return Action::None,
    };
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => {
            state.move_field(-1);
            Action::None
        }
        KeyCode::Down | KeyCode::Char('j') => {
            state.move_field(1);
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
