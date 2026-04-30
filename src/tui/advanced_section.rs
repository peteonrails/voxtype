//! Advanced settings: less-common knobs the TUI surfaces in one place.

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
pub struct AdvancedState {
    pub gpu_isolation: bool,
    pub on_demand_loading: bool,
    pub flash_attention: bool,
    pub eager_processing: bool,
    pub gpu_device: Option<i64>,
    pub field: Field,
    pub feedback: Option<(FeedbackLevel, String)>,
    pub dirty_since_load: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    GpuIsolation,
    OnDemand,
    FlashAttention,
    Eager,
    GpuDevice,
}
impl Field {
    const ALL: &'static [Field] = &[
        Field::GpuIsolation,
        Field::OnDemand,
        Field::FlashAttention,
        Field::Eager,
        Field::GpuDevice,
    ];
}

impl AdvancedState {
    pub fn load() -> Result<Self, EditorError> {
        let ed = ConfigEditor::load()?;
        Ok(Self {
            gpu_isolation: ed.get_bool("whisper", "gpu_isolation").unwrap_or(false),
            on_demand_loading: ed
                .get_bool("whisper", "on_demand_loading")
                .unwrap_or(false),
            flash_attention: ed.get_bool("whisper", "flash_attention").unwrap_or(false),
            eager_processing: ed
                .get_bool("whisper", "eager_processing")
                .unwrap_or(false),
            gpu_device: ed.get_int("whisper", "gpu_device"),
            field: Field::GpuIsolation,
            feedback: None,
            dirty_since_load: false,
        })
    }
    pub fn save(&mut self) -> Action {
        let mut ed = match ConfigEditor::load() {
            Ok(e) => e,
            Err(e) => {
                self.feedback = Some((FeedbackLevel::Err, format!("load: {}", e)));
                return Action::None;
            }
        };
        ed.set_bool("whisper", "gpu_isolation", self.gpu_isolation);
        ed.set_bool("whisper", "on_demand_loading", self.on_demand_loading);
        ed.set_bool("whisper", "flash_attention", self.flash_attention);
        ed.set_bool("whisper", "eager_processing", self.eager_processing);
        match self.gpu_device {
            Some(n) if n >= 0 => ed.set_int("whisper", "gpu_device", n),
            _ => ed.unset("whisper", "gpu_device"),
        }
        match ed.save() {
            Ok(()) => {
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
        if let Ok(fresh) = Self::load() {
            let field = self.field;
            *self = fresh;
            self.field = field;
            self.feedback = Some((FeedbackLevel::Ok, "Reverted".to_string()));
        }
    }
    fn move_field(&mut self, delta: i32) {
        let len = Field::ALL.len() as i32;
        let cur = Field::ALL.iter().position(|f| *f == self.field).unwrap_or(0) as i32;
        self.field = Field::ALL[((cur + delta).rem_euclid(len)) as usize];
    }
    fn cycle(&mut self, delta: i32) {
        match self.field {
            Field::GpuIsolation => self.gpu_isolation = !self.gpu_isolation,
            Field::OnDemand => self.on_demand_loading = !self.on_demand_loading,
            Field::FlashAttention => self.flash_attention = !self.flash_attention,
            Field::Eager => self.eager_processing = !self.eager_processing,
            Field::GpuDevice => {
                let cur = self.gpu_device.unwrap_or(-1);
                let next = cur + delta as i64;
                self.gpu_device = if next < 0 { None } else { Some(next.min(7)) };
            }
        }
        self.dirty_since_load = true;
        self.feedback = None;
    }
}

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let state = match &app.advanced {
        Some(s) => s,
        None => {
            let block = Block::default().borders(Borders::ALL).title("Advanced");
            let inner = block.inner(area);
            f.render_widget(block, area);
            f.render_widget(Paragraph::new("Failed to load config.").wrap(Wrap { trim: true }), inner);
            return;
        }
    };

    let rows = vec![
        FormRowSpec::new(
            state.field == Field::GpuIsolation,
            "GPU isolation (subprocess)",
            yesno(state.gpu_isolation),
        ),
        FormRowSpec::new(
            state.field == Field::OnDemand,
            "On-demand model loading",
            yesno(state.on_demand_loading),
        ),
        FormRowSpec::new(
            state.field == Field::FlashAttention,
            "Flash attention",
            yesno(state.flash_attention),
        ),
        FormRowSpec::new(
            state.field == Field::Eager,
            "Eager input processing",
            yesno(state.eager_processing),
        ),
        FormRowSpec::new(
            state.field == Field::GpuDevice,
            "GPU device index",
            state
                .gpu_device
                .map(|n| n.to_string())
                .unwrap_or_else(|| "auto".to_string()),
        ),
    ];

    let feedback_pair = state
        .feedback
        .as_ref()
        .map(|(lvl, msg)| (*lvl, msg.as_str()));

    common::render_form_with_guidance(
        f,
        area,
        "Advanced",
        state.dirty_since_load,
        feedback_pair,
        &rows,
        guidance_for_field(state),
    );
}

fn yesno(b: bool) -> String {
    (if b { "yes" } else { "no" }).to_string()
}

fn heading<'a>(text: &'a str) -> Line<'a> {
    Line::from(Span::styled(
        text,
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    ))
}

fn guidance_for_field(state: &AdvancedState) -> Vec<Line<'_>> {
    match state.field {
        Field::GpuIsolation => vec![
            heading("GPU isolation"),
            Line::from(""),
            Line::from(
                "Runs each transcription in a short-lived subprocess that \
                 exits afterward. The GPU releases all VRAM between recordings \
                 instead of holding the model resident.",
            ),
            Line::from(""),
            Line::from(Span::styled(
                "Turn it on if:",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from(
                "  • You're on a laptop with hybrid graphics and want the \
                 discrete GPU to power down between dictations.",
            ),
            Line::from(
                "  • You see VRAM use creep upward over a long voxtype \
                 session.",
            ),
            Line::from(""),
            Line::from(Span::styled(
                "Leave it off if:",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from(
                "  • Latency matters more than VRAM. Subprocess startup adds \
                 ~100-300 ms per recording.",
            ),
        ],
        Field::OnDemand => vec![
            heading("On-demand model loading"),
            Line::from(""),
            Line::from(
                "When on, voxtype loads the model only when recording starts \
                 (and unloads at idle). When off, the model stays resident \
                 from daemon start.",
            ),
            Line::from(""),
            Line::from(Span::styled(
                "Turn it on if:",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from(
                "  • You only dictate occasionally and don't want the daemon \
                 holding ~1-2 GB of RAM in the background.",
            ),
            Line::from(""),
            Line::from(Span::styled(
                "Leave it off if:",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from(
                "  • You dictate frequently. Resident-mode transcription \
                 starts instantly; on-demand loads add a one-shot delay on \
                 the first key press.",
            ),
        ],
        Field::FlashAttention => vec![
            heading("Flash attention"),
            Line::from(""),
            Line::from(
                "A GPU-only inference optimization that reduces memory \
                 bandwidth pressure in the attention layers. Speeds up \
                 transcription on capable cards.",
            ),
            Line::from(""),
            Line::from(Span::styled(
                "Turn it on if:",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from(
                "  • You're on Vulkan or CUDA with a recent GPU. \
                 Particularly noticeable on large-v3 and large-v3-turbo.",
            ),
            Line::from(""),
            Line::from(Span::styled(
                "Leave it off if:",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from(
                "  • You're CPU-only or on older hardware (no benefit, may \
                 cause crashes on a few drivers).",
            ),
        ],
        Field::Eager => vec![
            heading("Eager input processing"),
            Line::from(""),
            Line::from(
                "Voxtype starts transcribing audio chunks while you're still \
                 recording, instead of waiting until you release the PTT key. \
                 The final transcript stitches the chunks together.",
            ),
            Line::from(""),
            Line::from(Span::styled(
                "Turn it on if:",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from(
                "  • You record long-form (>10 sec) and the post-recording \
                 wait feels like dead time.",
            ),
            Line::from(""),
            Line::from(Span::styled(
                "Leave it off if:",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from(
                "  • You record short bursts (a few seconds). The chunked \
                 transcripts can occasionally split a sentence awkwardly.",
            ),
            Line::from(
                "  • You're on a laptop and CPU/GPU heat matters. Eager \
                 mode keeps the inference engine busy during recording.",
            ),
        ],
        Field::GpuDevice => vec![
            heading("GPU device index"),
            Line::from(""),
            Line::from(
                "Picks which GPU voxtype targets on multi-GPU systems. The \
                 default (auto) leaves the choice to the driver, which often \
                 picks the integrated GPU on hybrid laptops.",
            ),
            Line::from(""),
            Line::from(Span::styled(
                "Set a specific index if:",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from(
                "  • Transcription is slower than expected and you suspect \
                 the iGPU is being used. Try 1 (or 2) to target the \
                 discrete card.",
            ),
            Line::from(""),
            Line::from(Span::styled(
                "Run `vulkaninfo --summary` or `nvidia-smi -L` to see your \
                 device numbering.",
                Style::default().fg(Color::Gray),
            )),
        ],
    }
}

pub fn handle_key(app: &mut App, key: KeyEvent) -> Action {
    let state = match app.advanced.as_mut() {
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
