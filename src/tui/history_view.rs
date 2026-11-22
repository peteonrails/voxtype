//! A terminal history reader. It reads the same private day files as the CLI,
//! without initializing the configurator's hardware inventory or audio probes.

use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;

use crate::config::{self, HistoryConfig};
use crate::history::{index, HistoryEntry, HistoryIndex, HistoryQuery, HistoryStore};

enum Row {
    Dictation(Box<HistoryEntry>),
    Meeting(crate::meeting::search::SearchHit),
}

impl Row {
    fn id(&self) -> &str {
        match self {
            Self::Dictation(e) => &e.id,
            Self::Meeting(hit) => &hit.meeting_id,
        }
    }

    fn timestamp(&self) -> String {
        match self {
            Self::Dictation(e) => e.ts_start.to_rfc3339(),
            Self::Meeting(hit) => hit.date.clone(),
        }
    }

    fn detail(&self) -> String {
        match self {
            Self::Dictation(entry) => format!(
                "{}\n{}\n\nRaw:\n- {}\n\nOutput:\n+ {}\n\nCorrection:\n{}\n\nAutomatic edits:\n{}",
                entry.id,
                entry.outcome.as_str(),
                entry.raw_text,
                entry.text,
                entry.corrected_text.as_deref().unwrap_or("(none)"),
                serde_json::to_string_pretty(&entry.edits).unwrap_or_default()
            ),
            Self::Meeting(hit) => format!(
                "Meeting: {}\n{}\n{}\n\n{}",
                hit.title, hit.meeting_id, hit.date, hit.snippet_plain
            ),
        }
    }
}

struct Correction {
    id: String,
    text: String,
}

fn history_command(path: Option<&Path>, args: &[&str]) -> anyhow::Result<serde_json::Value> {
    let mut command = std::process::Command::new(std::env::current_exe()?);
    if let Some(path) = path {
        command.arg("--config").arg(path);
    }
    let output = command
        .args(["history", "--json"])
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()?;
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).map_err(|_| {
        anyhow::anyhow!(
            "History command did not return JSON: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )
    })?;
    if !output.status.success() {
        anyhow::bail!(
            "{}",
            value["error"].as_str().unwrap_or("History command failed")
        );
    }
    Ok(value)
}

struct View {
    settings: HistoryConfig,
    rows: Vec<Row>,
    selected: usize,
    expanded: bool,
    message: String,
    query: String,
    search_input: Option<String>,
    correction: Option<Correction>,
    delete_id: Option<String>,
    paste_id: Option<String>,
}

impl View {
    fn load(path: Option<&Path>) -> anyhow::Result<Self> {
        let config = config::load_config(path)?;
        Ok(Self {
            settings: config.history,
            rows: HistoryStore::at(HistoryStore::default_dir())
                .recent(200)?
                .into_iter()
                .map(|entry| Row::Dictation(Box::new(entry)))
                .collect(),
            selected: 0,
            expanded: false,
            message: String::new(),
            query: String::new(),
            search_input: None,
            correction: None,
            delete_id: None,
            paste_id: None,
        })
    }

    fn reload(&mut self, path: Option<&Path>) {
        let loaded = (|| -> anyhow::Result<Self> {
            let mut next = Self::load(path)?;
            next.query = self.query.clone();
            if !next.query.trim().is_empty() {
                let config = config::load_config(path)?;
                let store = HistoryStore::at(HistoryStore::default_dir());
                let db = index::db_path(&config);
                let index = HistoryIndex::open(&db)?;
                index.sync(&store)?;
                let meetings =
                    crate::meeting::MeetingStorage::open(crate::meeting::StorageConfig {
                        storage_path: db
                            .parent()
                            .ok_or_else(|| anyhow::anyhow!("Invalid shared history index path"))?
                            .to_path_buf(),
                        ..Default::default()
                    })?;
                meetings.backfill_search_index()?;
                let query = HistoryQuery {
                    text: next.query.clone(),
                    limit: 200,
                    since: None,
                };
                next.rows = index::search_everything(&index, &store, Some(&meetings), &query)?
                    .into_iter()
                    .map(|hit| match hit {
                        index::AnyHit::Dictation(hit) => Row::Dictation(Box::new(hit.entry)),
                        index::AnyHit::Meeting(hit) => Row::Meeting(hit),
                    })
                    .collect();
            }
            Ok(next)
        })();
        match loaded {
            Ok(mut next) => {
                let id = self.rows.get(self.selected).map(Row::id);
                next.selected = id
                    .and_then(|id| next.rows.iter().position(|e| e.id() == id))
                    .unwrap_or(0);
                next.expanded = self.expanded;
                next.search_input = self.search_input.take();
                next.correction = self.correction.take();
                next.delete_id = self.delete_id.take();
                next.paste_id = self.paste_id.take();
                *self = next;
            }
            Err(error) => self.message = error.to_string(),
        }
    }

    fn change_collection(&mut self, toggle_pause: bool, path: Option<&Path>) {
        let result = (|| -> anyhow::Result<()> {
            let current = config::load_config(path)?.history;
            let action = if toggle_pause {
                if current.paused {
                    "resume"
                } else {
                    "pause"
                }
            } else if current.enabled {
                "disable"
            } else {
                "enable"
            };
            history_command(path, &[action])?;
            Ok(())
        })();
        match result {
            Ok(()) => self.reload(path),
            Err(error) => self.message = format!("History change did not complete: {error}"),
        }
    }

    fn begin_correction(&mut self) {
        if let Some(Row::Dictation(entry)) = self.rows.get(self.selected) {
            self.correction = Some(Correction {
                id: entry.id.clone(),
                text: entry.best_text().to_string(),
            });
            self.message.clear();
        } else {
            self.message =
                "Select a dictation to correct; meeting search results are read-only here.".into();
        }
    }

    fn save_correction(&mut self, path: Option<&Path>) {
        let Some(correction) = &self.correction else {
            return;
        };
        match history_command(path, &["correct", &correction.id, "--", &correction.text]) {
            Ok(_) => {
                self.correction = None;
                self.reload(path);
                self.message = "Correction saved. Original text preserved.".into();
            }
            Err(error) => self.message = format!("Not saved: {error}"),
        }
    }

    fn copy_selected(&mut self, path: Option<&Path>) {
        let Some(Row::Dictation(entry)) = self.rows.get(self.selected) else {
            self.message = "Select a dictation to copy.".into();
            return;
        };
        if entry.best_text().is_empty() {
            self.message = "This transcript is empty.".into();
            return;
        }
        self.message = match history_command(path, &["copy", &entry.id]) {
            Ok(_) => "Copied transcript.".into(),
            Err(error) => format!("Copy did not complete: {error}"),
        };
    }

    fn begin_paste(&mut self) {
        let Some(Row::Dictation(entry)) = self.rows.get(self.selected) else {
            self.message = "Select a dictation to send again.".into();
            return;
        };
        if entry.best_text().is_empty() {
            self.message = "This transcript is empty.".into();
            return;
        }
        self.paste_id = Some(entry.id.clone());
    }

    fn begin_delete(&mut self) {
        if let Some(Row::Dictation(entry)) = self.rows.get(self.selected) {
            self.delete_id = Some(entry.id.clone());
        } else {
            self.message =
                "Select a dictation to delete; meeting search results are read-only here.".into();
        }
    }

    fn confirm_delete(&mut self, path: Option<&Path>) {
        let Some(id) = self.delete_id.take() else {
            return;
        };
        let result = history_command(path, &["delete", &id]);
        self.reload(path);
        self.message = match result {
            Ok(_) => "Dictation deleted.".into(),
            Err(error) => format!("Delete did not complete: {error}"),
        };
    }

    fn draw(&self, frame: &mut Frame) {
        let areas = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(2),
                Constraint::Length(if self.correction.is_some() { 5 } else { 4 }),
            ])
            .split(frame.area());
        let status = if !self.settings.enabled {
            "off"
        } else if self.settings.paused {
            "paused"
        } else {
            "on"
        };
        frame.render_widget(
            Paragraph::new(format!(
                "History: {status} | {} results | {}",
                self.rows.len(),
                self.query
            ))
            .block(Block::default().borders(Borders::ALL)),
            areas[0],
        );
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(if self.expanded {
                vec![Constraint::Percentage(45), Constraint::Percentage(55)]
            } else {
                vec![Constraint::Percentage(100)]
            })
            .split(areas[1]);
        let mut previous_day = None;
        let items = self
            .rows
            .iter()
            .map(|row| {
                let timestamp = row.timestamp();
                let day = timestamp.get(..10).unwrap_or("unknown date").to_string();
                let mut lines = Vec::new();
                if previous_day.as_ref() != Some(&day) {
                    lines.push(Line::styled(
                        day.to_string(),
                        Style::default().fg(Color::Cyan),
                    ));
                    previous_day = Some(day);
                }
                match row {
                    Row::Dictation(entry) => {
                        let failed = if entry.delivered { "" } else { " [failed]" };
                        let corrected = if entry.corrected_text.is_some() {
                            " [corrected]"
                        } else {
                            ""
                        };
                        lines.push(Line::from(format!(
                            "{}{failed}{corrected}",
                            entry.ts_start.format("%H:%M:%S")
                        )));
                        lines.push(Line::from(format!(
                            "{}  {}",
                            entry.app.as_deref().unwrap_or("unknown app"),
                            entry.model.as_str()
                        )));
                        lines.push(Line::from(entry.best_text().replace('\n', " ")));
                    }
                    Row::Meeting(hit) => {
                        lines.push(Line::from(format!("Meeting: {}", hit.title)));
                        lines.push(Line::from(hit.snippet_plain.replace('\n', " ")));
                    }
                }
                ListItem::new(lines)
            })
            .collect::<Vec<_>>();
        let mut selected = ListState::default();
        if !self.rows.is_empty() {
            selected.select(Some(self.selected));
        }
        let heading = if self.rows.is_empty() {
            if self.query.is_empty() {
                "No dictations stored"
            } else {
                "No matches"
            }
        } else if self.query.is_empty() {
            "Recent dictations"
        } else {
            "Dictations and meetings"
        };
        frame.render_stateful_widget(
            List::new(items)
                .block(Block::default().title(heading).borders(Borders::ALL))
                .highlight_style(Style::default().fg(Color::Yellow))
                .highlight_symbol("> "),
            columns[0],
            &mut selected,
        );
        if self.expanded {
            let text = self
                .rows
                .get(self.selected)
                .map(Row::detail)
                .unwrap_or_else(|| "No dictations stored.".into());
            frame.render_widget(
                Paragraph::new(text).wrap(Wrap { trim: false }).block(
                    Block::default()
                        .title("Dictation details")
                        .borders(Borders::ALL),
                ),
                columns[1],
            );
        }
        let help = if self.delete_id.is_some() {
            "Delete this dictation? y delete | n/Esc cancel".into()
        } else if self.paste_id.is_some() {
            "Send this transcript again? y closes reader; switch to target within 1.5 seconds | n/Esc cancel".into()
        } else if let Some(correction) = &self.correction {
            format!(
                "Correct transcript: {}\nEnter save | Esc cancel | Ctrl-U clear\n{}",
                correction.text, self.message
            )
        } else if let Some(input) = &self.search_input {
            format!("Search history and meetings: {input}")
        } else if self.message.is_empty() {
            "Arrows select | Enter edits | / search | e correct | d delete | c copy | a replay | o meeting | p pause/resume | h on/off | r refresh | q close".into()
        } else {
            self.message.clone()
        };
        frame.render_widget(
            Paragraph::new(help)
                .wrap(Wrap { trim: false })
                .block(Block::default().borders(Borders::ALL)),
            areas[2],
        );
    }
}

/// Open a private-file-only terminal reader; never probe microphones or restart
/// the daemon. JSON callers are handled before reaching this function.
pub fn run(config_path: Option<PathBuf>) -> anyhow::Result<()> {
    anyhow::ensure!(std::io::stdin().is_terminal() && std::io::stdout().is_terminal(), "History view needs a terminal. Use `voxtype history view --json` for noninteractive output.");
    let mut view = View::load(config_path.as_deref())?;
    let mut terminal = super::enter_terminal()?;
    let mut replay = None;
    let mut meeting = None;
    let result = (|| -> anyhow::Result<()> {
        let mut last_refresh = Instant::now();
        loop {
            terminal.draw(|frame| view.draw(frame))?;
            if event::poll(Duration::from_millis(250))? {
                if let Event::Key(key) = event::read()? {
                    if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
                        continue;
                    }
                    if key.modifiers.contains(KeyModifiers::CONTROL)
                        && key.code == KeyCode::Char('c')
                    {
                        break;
                    }
                    if view.delete_id.is_some() {
                        match key.code {
                            KeyCode::Char('y') => view.confirm_delete(config_path.as_deref()),
                            KeyCode::Char('n' | 'q') | KeyCode::Esc => view.delete_id = None,
                            _ => {}
                        }
                        continue;
                    }
                    if view.paste_id.is_some() {
                        match key.code {
                            KeyCode::Char('y') => {
                                replay = view.paste_id.take();
                                break;
                            }
                            KeyCode::Char('n' | 'q') | KeyCode::Esc => view.paste_id = None,
                            _ => {}
                        }
                        continue;
                    }
                    if let Some(correction) = &mut view.correction {
                        match key.code {
                            KeyCode::Esc => {
                                view.correction = None;
                                view.message.clear();
                            }
                            KeyCode::Enter => view.save_correction(config_path.as_deref()),
                            KeyCode::Backspace => {
                                correction.text.pop();
                            }
                            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                correction.text.clear()
                            }
                            KeyCode::Char(c) => correction.text.push(c),
                            _ => {}
                        }
                        continue;
                    }
                    if let Some(input) = &mut view.search_input {
                        match key.code {
                            KeyCode::Esc => view.search_input = None,
                            KeyCode::Enter => {
                                view.query = view.search_input.take().unwrap_or_default();
                                view.reload(config_path.as_deref());
                            }
                            KeyCode::Backspace => {
                                input.pop();
                            }
                            KeyCode::Char(c) => input.push(c),
                            _ => {}
                        }
                        continue;
                    }
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => break,
                        KeyCode::Up => view.selected = view.selected.saturating_sub(1),
                        KeyCode::Down => {
                            view.selected =
                                (view.selected + 1).min(view.rows.len().saturating_sub(1))
                        }
                        KeyCode::Enter => view.expanded = !view.expanded,
                        KeyCode::Char('r') => view.reload(config_path.as_deref()),
                        KeyCode::Char('/') => view.search_input = Some(view.query.clone()),
                        KeyCode::Char('e') => view.begin_correction(),
                        KeyCode::Char('d') => view.begin_delete(),
                        KeyCode::Char('c') => view.copy_selected(config_path.as_deref()),
                        KeyCode::Char('a') => view.begin_paste(),
                        KeyCode::Char('o') => {
                            if let Some(Row::Meeting(hit)) = view.rows.get(view.selected) {
                                meeting = Some(hit.meeting_id.clone());
                                break;
                            }
                            view.message = "Select a meeting search result to open.".into();
                        }
                        KeyCode::Char('p') => view.change_collection(true, config_path.as_deref()),
                        KeyCode::Char('h') => view.change_collection(false, config_path.as_deref()),
                        _ => {}
                    }
                }
            }
            if last_refresh.elapsed() >= Duration::from_secs(2) {
                view.reload(config_path.as_deref());
                last_refresh = Instant::now();
            }
        }
        Ok(())
    })();
    let cleanup = super::leave_terminal(&mut terminal);
    result.and(cleanup)?;
    if let Some(id) = replay {
        eprintln!(
            "History reader closed. Switch to the target application; sending in 1.5 seconds."
        );
        history_command(
            config_path.as_deref(),
            &["paste", &id, "--delay-ms", "1500"],
        )
        .map_err(|error| anyhow::anyhow!("Replay did not complete: {error}"))?;
    }
    if let Some(id) = meeting {
        let mut command = std::process::Command::new(std::env::current_exe()?);
        if let Some(path) = &config_path {
            command.arg("--config").arg(path);
        }
        let status = command.args(["meeting", "view", &id]).status()?;
        anyhow::ensure!(status.success(), "Could not open the selected meeting view");
    }
    Ok(())
}
