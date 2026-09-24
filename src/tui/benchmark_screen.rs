//! Benchmark screen for `voxtype configure` (press `b` on General).
//!
//! Recording runs in a `voxtype setup benchmark --record-to` child, the same
//! out-of-process approach the Audio section uses for device probing, so a
//! stuck or crashing audio device cannot take the TUI down (#541). The
//! benchmark itself is the shared library in `setup::benchmark`, run on a
//! background thread that reports progress back to this screen.

use crate::setup::benchmark::{self, BenchEvent, BenchmarkInput, Report};
use crate::setup::binary::{self, InstallKind, Variant};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use super::app::{Action, App};

/// How long the recorder gets to write the WAV after being told to stop.
const STOP_TIMEOUT: Duration = Duration::from_secs(10);

pub enum Message {
    Event(BenchEvent),
    Done {
        result: Result<Report, String>,
        /// `None` when there was nothing worth saving (no recommendation).
        saved: Option<Result<(), String>>,
    },
}

pub enum Phase {
    Intro,
    Recording {
        child: Child,
        started: Instant,
    },
    Stopping {
        child: Child,
        deadline: Instant,
    },
    Benchmarking {
        rx: mpsc::Receiver<Message>,
        cancel: Arc<AtomicBool>,
        log: Vec<String>,
    },
    Done {
        report: Report,
        saved: Option<Result<(), String>>,
    },
    Failed(String),
}

pub struct BenchmarkScreen {
    pub phase: Phase,
    engine: String,
    candidates: Vec<Variant>,
    warnings: Vec<String>,
    config_path: Option<PathBuf>,
    recording: Option<tempfile::NamedTempFile>,
}

impl BenchmarkScreen {
    pub fn open() -> Self {
        let config_path = super::config_editor::tui_config_path();
        let engine = crate::config::load_config(config_path.as_deref())
            .map(|c| c.engine.name().to_string())
            .unwrap_or_else(|_| "whisper".to_string());
        let candidates = benchmark::candidates(&engine);
        let phase = if candidates.is_empty() {
            Phase::Failed(format!(
                "No installed variant in {} can run the {} engine on this machine, so there \
                 is nothing to benchmark.",
                binary::LIB_DIR,
                engine
            ))
        } else {
            Phase::Intro
        };
        Self {
            phase,
            engine,
            candidates,
            warnings: benchmark::warnings(),
            config_path,
            recording: None,
        }
    }

    fn start_recording(&mut self) {
        let file = match tempfile::Builder::new()
            .prefix("voxtype-benchmark-")
            .suffix(".wav")
            .tempfile()
        {
            Ok(file) => file,
            Err(e) => {
                self.phase = Phase::Failed(format!("Cannot create the recording file: {}", e));
                return;
            }
        };
        let exe = match std::env::current_exe() {
            Ok(exe) => exe,
            Err(e) => {
                self.phase = Phase::Failed(format!("Cannot find the voxtype binary: {}", e));
                return;
            }
        };
        let mut cmd = Command::new(exe);
        if let Some(path) = &self.config_path {
            cmd.arg("--config").arg(path);
        }
        cmd.args(["setup", "benchmark", "--record-to"])
            .arg(file.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        match cmd.spawn() {
            Ok(child) => {
                self.recording = Some(file);
                self.phase = Phase::Recording {
                    child,
                    started: Instant::now(),
                };
            }
            Err(e) => self.phase = Phase::Failed(format!("Cannot start the recorder: {}", e)),
        }
    }

    fn stop_recording(&mut self) {
        if let Phase::Recording { mut child, .. } = std::mem::replace(&mut self.phase, Phase::Intro)
        {
            // Closing stdin stops the recorder, which treats end of file like a
            // newline. Writing to it instead would raise SIGPIPE and kill the
            // TUI if the recorder had already exited.
            drop(child.stdin.take());
            self.phase = Phase::Stopping {
                child,
                deadline: Instant::now() + STOP_TIMEOUT,
            };
        }
    }

    /// Advance background work. Returns the report once a benchmark finishes.
    pub fn poll(&mut self) -> Option<Report> {
        enum Next {
            Nothing,
            RecorderExited,
            RecorderStuck,
            Messages(Vec<Message>),
        }
        let next = match &mut self.phase {
            // The recorder also stops by itself after MAX_RECORDING_SECS.
            Phase::Recording { child, .. } => match child.try_wait() {
                Ok(None) => Next::Nothing,
                _ => Next::RecorderExited,
            },
            Phase::Stopping { child, deadline } => match child.try_wait() {
                Ok(None) if Instant::now() >= *deadline => Next::RecorderStuck,
                Ok(None) => Next::Nothing,
                _ => Next::RecorderExited,
            },
            Phase::Benchmarking { rx, .. } => Next::Messages(rx.try_iter().collect()),
            _ => Next::Nothing,
        };

        match next {
            Next::Nothing => None,
            Next::RecorderExited => {
                if let Phase::Recording { child, .. } | Phase::Stopping { child, .. } =
                    std::mem::replace(&mut self.phase, Phase::Intro)
                {
                    self.finish_recording(child);
                }
                None
            }
            Next::RecorderStuck => {
                if let Phase::Stopping { mut child, .. } =
                    std::mem::replace(&mut self.phase, Phase::Intro)
                {
                    let _ = child.kill();
                    let _ = child.wait();
                }
                self.phase = Phase::Failed(
                    "The recorder did not stop in time; the audio device may be stuck.".to_string(),
                );
                None
            }
            Next::Messages(messages) => {
                let mut outcome = None;
                if let Phase::Benchmarking { log, .. } = &mut self.phase {
                    for message in messages {
                        match message {
                            Message::Event(event) => log.push(event.describe()),
                            Message::Done { result, saved } => outcome = Some((result, saved)),
                        }
                    }
                }
                match outcome {
                    Some((Ok(report), saved)) => {
                        // Only a saved report may replace what General shows.
                        let shown = matches!(saved, Some(Ok(()))).then(|| report.clone());
                        self.phase = Phase::Done { report, saved };
                        shown
                    }
                    Some((Err(e), _)) => {
                        self.phase = Phase::Failed(e);
                        None
                    }
                    None => None,
                }
            }
        }
    }

    fn finish_recording(&mut self, child: Child) {
        let output = match child.wait_with_output() {
            Ok(output) => output,
            Err(e) => {
                self.phase = Phase::Failed(format!("The recorder failed: {}", e));
                return;
            }
        };
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let message = stderr
                .lines()
                .rev()
                .map(crate::setup::accel::strip_ansi)
                .find(|l| !l.trim().is_empty())
                .unwrap_or_else(|| format!("The recorder exited with {}", output.status));
            self.phase = Phase::Failed(message);
            return;
        }
        let Some(file) = self.recording.take() else {
            self.phase = Phase::Failed("The recording went missing.".to_string());
            return;
        };

        let input = BenchmarkInput {
            engine: self.engine.clone(),
            candidates: self.candidates.clone(),
            wav: file.path().to_path_buf(),
            reference: Some(benchmark::PASSAGE.to_string()),
            runs: benchmark::DEFAULT_RUNS,
            config_path: self.config_path.clone(),
            lib_dir: PathBuf::from(binary::LIB_DIR),
        };
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let thread_cancel = cancel.clone();
        std::thread::spawn(move || {
            // The thread owns the recording so it outlives a closed screen.
            let _file = file;
            let events = tx.clone();
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                benchmark::run_benchmark(
                    &input,
                    &mut |event| {
                        let _ = events.send(Message::Event(event));
                    },
                    &thread_cancel,
                )
            }))
            .unwrap_or_else(|_| Err(anyhow::anyhow!("The benchmark crashed unexpectedly.")))
            .map_err(|e| e.to_string());
            // A run without a recommendation must not replace earlier results.
            let saved = match &result {
                Ok(report) if report.recommended.is_some() => Some(
                    benchmark::save_report(report)
                        .map(|_| ())
                        .map_err(|e| e.to_string()),
                ),
                _ => None,
            };
            let _ = tx.send(Message::Done { result, saved });
        });
        self.phase = Phase::Benchmarking {
            rx,
            cancel,
            log: Vec::new(),
        };
    }

    /// Stop whatever is running. Also called when the screen is dropped.
    fn close(&mut self) {
        match &mut self.phase {
            Phase::Recording { child, .. } | Phase::Stopping { child, .. } => {
                let _ = child.kill();
                let _ = child.wait();
            }
            Phase::Benchmarking { cancel, .. } => cancel.store(true, Ordering::Relaxed),
            _ => {}
        }
    }
}

impl Drop for BenchmarkScreen {
    fn drop(&mut self) {
        self.close();
    }
}

/// The variant Enter would switch to on the results screen, if switching to it
/// makes sense here.
fn switch_target(report: &Report, inv: &binary::Inventory) -> Option<Variant> {
    report
        .recommended
        .filter(|&v| inv.install_kind == InstallKind::Package && inv.active_variant != Some(v))
}

pub fn handle_key(app: &mut App, key: KeyEvent) -> Action {
    let Some(screen) = app.benchmark.as_mut() else {
        return Action::None;
    };
    let ctrl_c = key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        _ if ctrl_c => {
            app.benchmark = None;
            Action::None
        }
        KeyCode::Esc | KeyCode::Char('q') => {
            app.benchmark = None;
            Action::None
        }
        KeyCode::Enter => match &screen.phase {
            Phase::Intro => {
                screen.start_recording();
                Action::None
            }
            Phase::Recording { .. } => {
                screen.stop_recording();
                Action::None
            }
            Phase::Done { report, .. } => match switch_target(report, &app.inventory) {
                Some(variant) => {
                    app.benchmark = None;
                    Action::SwitchVariant(variant)
                }
                None => Action::None,
            },
            _ => Action::None,
        },
        _ => Action::None,
    }
}

pub fn render(f: &mut Frame, screen: &BenchmarkScreen, app: &App) {
    let area = f.area();
    let w = area.width.saturating_sub(6).min(90);
    let h = area.height.saturating_sub(4).min(28);
    let rect = Rect {
        x: area.x + area.width.saturating_sub(w) / 2,
        y: area.y + area.height.saturating_sub(h) / 2,
        width: w,
        height: h,
    };
    f.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(" Benchmark installed builds ");
    let inner = block.inner(rect);
    f.render_widget(block, rect);

    let bold = Style::default().add_modifier(Modifier::BOLD);
    let dim = Style::default().fg(Color::Gray);
    let keys = Style::default().fg(Color::Cyan);
    let warn = Style::default().fg(Color::Yellow);

    let passage = |lines: &mut Vec<Line>| {
        lines.push(Line::from(Span::styled(
            "Read this passage aloud at your normal pace and volume:",
            bold,
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(format!("  {}", benchmark::PASSAGE)));
        lines.push(Line::from(""));
    };

    let mut lines: Vec<Line> = Vec::new();
    match &screen.phase {
        Phase::Intro => {
            passage(&mut lines);
            let names: Vec<&str> = screen.candidates.iter().map(|v| v.display()).collect();
            lines.push(Line::from(Span::styled(
                format!("Builds to test: {}", names.join(", ")),
                dim,
            )));
            lines.push(Line::from(Span::styled(
                format!(
                    "Each gets one warm-up run and {} timed runs. A build already slower than an \
                     accurate one is stopped early.",
                    benchmark::DEFAULT_RUNS
                ),
                dim,
            )));
            for w in &screen.warnings {
                lines.push(Line::from(Span::styled(format!("! {}", w), warn)));
            }
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "Enter start recording · Esc cancel",
                keys,
            )));
        }
        Phase::Recording { started, .. } => {
            passage(&mut lines);
            lines.push(Line::from(Span::styled(
                format!(
                    "● Recording {}s. Press Enter when you finish reading.",
                    started.elapsed().as_secs()
                ),
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled("Esc cancel", keys)));
        }
        Phase::Stopping { .. } => {
            lines.push(Line::from(Span::styled("Finishing the recording...", bold)));
        }
        Phase::Benchmarking { log, .. } => {
            lines.push(Line::from(Span::styled(
                "Benchmarking. Slower builds are stopped as soon as they fall behind.",
                bold,
            )));
            lines.push(Line::from(""));
            let room = usize::from(inner.height).saturating_sub(4);
            let skip = log.len().saturating_sub(room);
            for entry in &log[skip..] {
                lines.push(Line::from(entry.clone()));
            }
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled("Esc cancel", keys)));
        }
        Phase::Done { report, saved } => {
            for line in benchmark::report_lines(report) {
                lines.push(Line::from(line));
            }
            lines.push(match saved {
                Some(Ok(())) => Line::from(Span::styled(
                    "Saved. The General screen now stars the measured pick.",
                    dim,
                )),
                Some(Err(e)) => Line::from(Span::styled(
                    format!("Could not save the results: {}", e),
                    warn,
                )),
                None => Line::from(Span::styled(
                    "Nothing was saved, so any earlier results are kept.",
                    dim,
                )),
            });
            lines.push(Line::from(""));
            let hint = match switch_target(report, &app.inventory) {
                Some(v) => format!("Enter switch to {} · Esc close", v.display()),
                None => "Esc close".to_string(),
            };
            lines.push(Line::from(Span::styled(hint, keys)));
        }
        Phase::Failed(message) => {
            lines.push(Line::from(Span::styled(
                "Benchmark failed",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(""));
            lines.push(Line::from(message.clone()));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled("Esc close", keys)));
        }
    }
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::setup::benchmark::VariantResult;
    use crossterm::event::KeyModifiers;

    fn screen(phase: Phase) -> BenchmarkScreen {
        BenchmarkScreen {
            phase,
            engine: "whisper".to_string(),
            candidates: vec![Variant::WhisperVulkan, Variant::WhisperNative],
            warnings: Vec::new(),
            config_path: None,
            recording: None,
        }
    }

    fn done(recommended: Variant) -> Phase {
        let result: VariantResult = serde_json::from_value(serde_json::json!({
            "variant": "whisper-vulkan",
            "binary_name": "voxtype-vulkan",
            "runs_secs": [0.6],
            "median_secs": 0.6,
            "model_load_secs": 0.4,
            "word_error_rate": 0.0,
            "transcript": "text",
            "error": null
        }))
        .unwrap();
        Phase::Done {
            report: Report {
                measured_at: 0,
                engine: "whisper".to_string(),
                audio_secs: 12.0,
                scored: true,
                results: vec![result],
                recommended: Some(recommended),
                warnings: Vec::new(),
            },
            saved: Some(Ok(())),
        }
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    /// The TUI opens with the sidebar focused; `b` must still start the
    /// benchmark from General instead of being swallowed by the sidebar.
    #[test]
    fn b_opens_the_benchmark_with_the_sidebar_focused() {
        let mut app = App::new(false);
        app.current_section = super::super::section::Section::General;
        app.sidebar_focused = true;
        let action = super::super::handle_global_key(&mut app, key(KeyCode::Char('b')));
        assert!(matches!(action, Some(Action::None)));
        assert!(app.benchmark.is_some());
    }

    #[test]
    fn escape_closes_the_screen() {
        let mut app = App::new(false);
        app.benchmark = Some(screen(Phase::Intro));
        assert!(matches!(
            handle_key(&mut app, key(KeyCode::Esc)),
            Action::None
        ));
        assert!(app.benchmark.is_none());
    }

    #[test]
    fn ctrl_c_closes_the_screen() {
        let mut app = App::new(false);
        app.benchmark = Some(screen(Phase::Intro));
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert!(matches!(handle_key(&mut app, ctrl_c), Action::None));
        assert!(app.benchmark.is_none());
    }

    /// A report that failed to save, or was not worth saving, must not move
    /// the General screen's star: the next refresh would drop it again.
    #[test]
    fn only_a_saved_report_reaches_the_general_screen() {
        let Phase::Done { report, .. } = done(Variant::WhisperVulkan) else {
            unreachable!()
        };
        let cases = [
            (Some(Ok(())), true),
            (Some(Err("disk full".to_string())), false),
            (None, false),
        ];
        for (saved, shown) in cases {
            let (tx, rx) = mpsc::channel();
            let mut s = screen(Phase::Benchmarking {
                rx,
                cancel: Arc::new(AtomicBool::new(false)),
                log: Vec::new(),
            });
            tx.send(Message::Done {
                result: Ok(report.clone()),
                saved: saved.clone(),
            })
            .unwrap();
            assert_eq!(s.poll().is_some(), shown);
            assert!(matches!(&s.phase, Phase::Done { saved: got, .. } if *got == saved));
        }
    }

    #[test]
    fn enter_on_results_switches_to_the_measured_pick() {
        let mut app = App::new(false);
        app.inventory.install_kind = InstallKind::Package;
        app.inventory.active_variant = Some(Variant::WhisperNative);
        app.benchmark = Some(screen(done(Variant::WhisperVulkan)));
        assert!(matches!(
            handle_key(&mut app, key(KeyCode::Enter)),
            Action::SwitchVariant(Variant::WhisperVulkan)
        ));
        assert!(app.benchmark.is_none());
    }

    #[test]
    fn enter_on_results_does_nothing_when_already_active() {
        let mut app = App::new(false);
        app.inventory.install_kind = InstallKind::Package;
        app.inventory.active_variant = Some(Variant::WhisperVulkan);
        app.benchmark = Some(screen(done(Variant::WhisperVulkan)));
        assert!(matches!(
            handle_key(&mut app, key(KeyCode::Enter)),
            Action::None
        ));
        assert!(app.benchmark.is_some());
    }
}
