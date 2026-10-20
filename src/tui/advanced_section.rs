//! Advanced: eager processing, the GPU to use, and streaming output.
//!
//! Whisper's GPU isolation, flash attention and on-demand loading live on
//! the Engine page with the rest of Whisper's settings. They used to be
//! here as well, and two pages holding the same setting could each save
//! over the other's edit.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::Rect,
    text::{Line, Span},
    Frame,
};

use super::app::{Action, App};
use super::common::{self, FeedbackLevel, FormRowSpec};
use super::config_editor::{ConfigEditor, EditorError};
use super::theme;

#[derive(Debug, Clone)]
pub struct AdvancedState {
    pub eager_processing: bool,
    pub gpu_device: Option<i64>,
    pub streaming: bool,
    pub field: Field,
    pub feedback: Option<(FeedbackLevel, String)>,
    pub dirty_since_load: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Eager,
    GpuDevice,
    Streaming,
}

impl Field {
    const ALL: &'static [Field] = &[Field::Eager, Field::GpuDevice, Field::Streaming];
}

impl AdvancedState {
    pub fn load() -> Result<Self, EditorError> {
        let ed = ConfigEditor::load()?;
        Ok(Self {
            eager_processing: ed.get_bool("whisper", "eager_processing").unwrap_or(false),
            gpu_device: ed.get_int("whisper", "gpu_device"),
            streaming: ed.get_bool("parakeet", "streaming").unwrap_or(false),
            field: Field::Eager,
            feedback: None,
            dirty_since_load: false,
        })
    }

    /// Save. Turning streaming on also makes the hotkey toggle and, when
    /// the Parakeet model can't stream, switches to one that can and asks
    /// for it to be downloaded, so voxtype works after the restart.
    pub fn save(&mut self) -> Action {
        let mut ed = match ConfigEditor::load() {
            Ok(e) => e,
            Err(e) => {
                self.feedback = Some((
                    FeedbackLevel::Err,
                    format!("Couldn't read the config: {}", e),
                ));
                return Action::None;
            }
        };
        ed.set_bool("whisper", "eager_processing", self.eager_processing);
        match self.gpu_device {
            Some(n) if n >= 0 => ed.set_int("whisper", "gpu_device", n),
            _ => ed.unset("whisper", "gpu_device"),
        }
        let was_streaming = ed.get_bool("parakeet", "streaming").unwrap_or(false);
        ed.set_bool("parakeet", "streaming", self.streaming);

        // Streaming types while the key would still be held, which confuses
        // the held-key tracking of Hyprland, Sway and River, so it needs
        // toggle activation.
        if self.streaming
            && ed
                .get_string("hotkey", "mode")
                .is_some_and(|m| m == "push_to_talk")
        {
            ed.set_string("hotkey", "mode", "toggle");
        }

        // Streaming needs a model that ships tokenizer.model (#423).
        let mut download = None;
        if self.streaming && !was_streaming {
            let current = ed
                .get_string("parakeet", "model")
                .unwrap_or_else(|| "parakeet-tdt-0.6b-v3".to_string());
            if !crate::setup::model::is_streaming_compatible_parakeet(&current) {
                let model = crate::setup::model::DEFAULT_PARAKEET_STREAMING_MODEL;
                ed.set_string("parakeet", "model", model);
                if !crate::model_catalog::model_installed("parakeet", model) {
                    download = Some(model.to_string());
                }
            }
        }

        match ed.save() {
            Ok(()) => {
                self.dirty_since_load = false;
                self.feedback = None;
                match download {
                    Some(model) => Action::DownloadModel {
                        engine: "parakeet".to_string(),
                        model,
                    },
                    None => Action::None,
                }
            }
            Err(e) => {
                self.feedback = Some((FeedbackLevel::Err, format!("Not saved: {}", e)));
                Action::None
            }
        }
    }

    pub fn reset(&mut self) {
        if let Ok(fresh) = Self::load() {
            let field = self.field;
            *self = fresh;
            self.field = field;
            self.feedback = Some((FeedbackLevel::Ok, "Put back the saved settings".to_string()));
        }
    }

    fn move_field(&mut self, delta: i32) {
        let len = Field::ALL.len() as i32;
        let cur = Field::ALL
            .iter()
            .position(|f| *f == self.field)
            .unwrap_or(0) as i32;
        self.field = Field::ALL[((cur + delta).rem_euclid(len)) as usize];
    }

    fn cycle(&mut self, delta: i32) {
        match self.field {
            Field::Eager => self.eager_processing = !self.eager_processing,
            Field::GpuDevice => {
                let cur = self.gpu_device.unwrap_or(-1);
                let next = cur + delta as i64;
                self.gpu_device = if next < 0 { None } else { Some(next.min(7)) };
            }
            Field::Streaming => self.streaming = !self.streaming,
        }
        self.dirty_since_load = true;
        self.feedback = None;
    }
}

/// The page's rows, from its in-memory settings.
pub fn rows(state: &AdvancedState) -> Vec<FormRowSpec> {
    vec![
        FormRowSpec::new(
            state.field == Field::Eager,
            "Transcribe while recording",
            common::on_off(state.eager_processing),
        ),
        FormRowSpec::new(
            state.field == Field::GpuDevice,
            "GPU",
            state
                .gpu_device
                .map(|n| format!("Number {}", n))
                .unwrap_or_else(|| "Automatic".to_string()),
        ),
        FormRowSpec::new(
            state.field == Field::Streaming,
            "Streaming output",
            common::on_off(state.streaming),
        ),
    ]
}

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let Some(state) = &app.advanced else {
        common::render_load_error(f, area, app);
        return;
    };
    let feedback_pair = state
        .feedback
        .as_ref()
        .map(|(lvl, msg)| (*lvl, msg.as_str()));
    common::render_form_with_guidance(f, area, app, feedback_pair, &rows(state), guidance(state));
}

fn guidance(state: &AdvancedState) -> Vec<Line<'static>> {
    let text = |s: &'static str| Line::from(s);
    let dim = |s: &'static str| Line::from(Span::styled(s, theme::muted()));
    match state.field {
        Field::Eager => vec![
            text(
                "Starts transcribing chunks while you are still talking, so long \
                 recordings finish sooner. Short ones gain nothing.",
            ),
            dim("Keeps the CPU or GPU busy while recording."),
        ],
        Field::GpuDevice => vec![
            text(
                "Which graphics card to use when there are several. Automatic \
                 often picks the built-in one on laptops; try 1 for the other.",
            ),
            dim("vulkaninfo --summary or nvidia-smi -L lists them."),
        ],
        Field::Streaming => vec![
            text(
                "Types words as you speak instead of after you stop. Parakeet \
                 only, and experimental.",
            ),
            dim(
                "Saving with it on sets the hotkey to toggle and, if needed, \
                 switches to a Parakeet model that can stream and downloads it.",
            ),
        ],
    }
}

pub fn handle_key(app: &mut App, key: KeyEvent) -> Action {
    let Some(state) = app.advanced.as_mut() else {
        return Action::None;
    };
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => state.move_field(-1),
        KeyCode::Down | KeyCode::Char('j') => state.move_field(1),
        KeyCode::Left | KeyCode::Char('h') => state.cycle(-1),
        KeyCode::Right | KeyCode::Char('l') | KeyCode::Char(' ') => state.cycle(1),
        KeyCode::Char('r') => state.reset(),
        _ => {}
    }
    Action::None
}
