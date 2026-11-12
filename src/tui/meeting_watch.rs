//! `voxtype meeting watch`: a meeting in the terminal.
//!
//! Draws [`crate::meeting::view::MeetingView`], fed by the daemon's
//! `meeting.sock`, with ratatui. Every action runs the matching `voxtype
//! meeting` command (pause, stop, label, reassign, annotate, ask) as a child
//! process, so it gets the same checks and messages as typing it, and the
//! screen never blocks on one.
//!
//! With no meeting running it lists recent meetings; Enter opens one as
//! ended, Esc goes back.

use crate::meeting::data::TranscriptSegment;
use crate::meeting::storage::MeetingListEntry;
use crate::meeting::view::client::{spawn_feed, Feed};
use crate::meeting::view::{clock, meter, Change, LineMarks, MeetingView, SpeakerChip, ViewState};
use crate::meeting::MeetingConfig;
use chrono::{DateTime, Utc};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;
use std::io::BufRead;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

/// Where to find things.
pub struct WatchOptions {
    /// The voxtype binary actions run (normally the running one).
    pub exe: PathBuf,
    /// Extra arguments placed before the subcommand (`--config <path>`).
    pub global_args: Vec<String>,
    pub socket: PathBuf,
    pub meetings: MeetingConfig,
}

const PALETTE: [Color; 8] = [
    Color::Cyan,
    Color::Magenta,
    Color::Yellow,
    Color::Green,
    Color::LightBlue,
    Color::LightRed,
    Color::LightGreen,
    Color::LightMagenta,
];

/// What is on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    List,
    Meeting,
}

/// A dialog over the meeting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Modal {
    None,
    ConfirmStop,
    /// Pick a speaker chip, then type the name to give it.
    Label {
        chips: Vec<SpeakerChip>,
        selected: usize,
        typing: Option<String>,
    },
    /// Pick a speaker for the selected line; the last choice types a new one.
    Reassign {
        choices: Vec<String>,
        selected: usize,
        typing: Option<String>,
    },
    Note(String),
    Ask(String),
}

/// The ask pane under the transcript.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AskPane {
    pub question: String,
    pub answer: String,
    pub agent_name: Option<String>,
    pub done: bool,
    pub error: Option<String>,
}

/// Results coming back from child processes.
#[derive(Debug)]
pub enum Outcome {
    Done {
        what: String,
        ok: bool,
        message: String,
        reload: bool,
        /// Speaker keys and the name `meeting label` gave them, for the
        /// live view to apply (the socket does not carry labels).
        label: Option<(Vec<String>, String)>,
    },
    AskLine(String),
    AskExited(bool),
}

/// The watch screen's state. Drawing reads it; keys and feed events change it.
pub struct WatchApp {
    pub screen: Screen,
    /// Follows the socket, always.
    pub live: MeetingView,
    /// A meeting opened from the list.
    pub stored: Option<MeetingView>,
    pub list: Vec<MeetingListEntry>,
    pub list_selected: usize,
    /// Index into the shown view's lines.
    pub selected: Option<usize>,
    /// Keep the newest line selected and in view.
    pub follow: bool,
    pub modal: Modal,
    pub ask: Option<AskPane>,
    pub message: Option<String>,
    /// First transcript row on screen, kept between frames.
    scroll: usize,
    /// No `hello` yet: the first one decides the starting screen.
    pub starting: bool,
    pub quit: bool,
}

impl Default for WatchApp {
    fn default() -> Self {
        Self::new()
    }
}

impl WatchApp {
    pub fn new() -> Self {
        Self {
            screen: Screen::List,
            live: MeetingView::live(),
            stored: None,
            list: Vec::new(),
            list_selected: 0,
            selected: None,
            follow: true,
            modal: Modal::None,
            ask: None,
            message: None,
            scroll: 0,
            starting: true,
            quit: false,
        }
    }

    /// The meeting shown on the Meeting screen.
    pub fn view(&self) -> &MeetingView {
        self.stored.as_ref().unwrap_or(&self.live)
    }

    /// Whether the list has a "live meeting" first row.
    fn has_live_row(&self) -> bool {
        self.live.state.is_live() || self.live.state == ViewState::Summarizing
    }

    fn list_len(&self) -> usize {
        self.list.len() + usize::from(self.has_live_row())
    }

    /// Feed one socket message in. Returns true when the feed should
    /// reconnect for a fresh snapshot.
    pub fn on_feed(&mut self, feed: Feed) -> bool {
        match feed {
            Feed::Connected => false,
            Feed::Disconnected => {
                self.live.disconnected();
                if self.starting {
                    // No daemon: nothing live, start on the list.
                    self.starting = false;
                }
                false
            }
            Feed::Line(line) => {
                let change = self.live.apply_line(&line);
                if let Some(stored) = self.stored.as_mut() {
                    stored.apply_line(&line);
                }
                if self.starting && line.contains("\"hello\"") {
                    self.starting = false;
                    if self.live.state.is_live() {
                        self.screen = Screen::Meeting;
                    }
                }
                if change == Change::Reset && self.stored.is_none() {
                    self.selected = None;
                    self.follow = true;
                }
                self.keep_selection_in_range();
                change == Change::Resync
            }
        }
    }

    fn keep_selection_in_range(&mut self) {
        let n = self.view().lines().len();
        if n == 0 {
            self.selected = None;
        } else if self.follow {
            self.selected = Some(n - 1);
        } else if let Some(s) = self.selected {
            self.selected = Some(s.min(n - 1));
        }
    }

    /// Show the meeting chosen in the list. `load` reads a stored meeting.
    fn open_selected(&mut self, load: &dyn Fn(&str) -> Option<MeetingView>) {
        let live_row = self.has_live_row();
        if live_row && self.list_selected == 0 {
            self.stored = None;
        } else {
            let at = self.list_selected - usize::from(live_row);
            let Some(entry) = self.list.get(at) else {
                return;
            };
            match load(&entry.metadata.id.to_string()) {
                Some(view) => self.stored = Some(view),
                None => {
                    self.message = Some("Could not load that meeting".into());
                    return;
                }
            }
        }
        self.screen = Screen::Meeting;
        self.follow = false;
        self.selected = if self.view().lines().is_empty() {
            None
        } else if self.stored.is_some() {
            Some(0)
        } else {
            self.follow = true;
            Some(self.view().lines().len() - 1)
        };
        self.scroll = 0;
        self.ask = None;
    }

    fn selected_segment(&self) -> Option<&TranscriptSegment> {
        self.view().lines().get(self.selected?)
    }

    fn move_selection(&mut self, delta: i64) {
        let n = self.view().lines().len();
        if n == 0 {
            return;
        }
        let cur = self.selected.unwrap_or(n - 1) as i64;
        let next = (cur + delta).clamp(0, n as i64 - 1) as usize;
        self.selected = Some(next);
        // Scrolling up stops following; reaching the newest line resumes.
        self.follow = next == n - 1 && delta > 0;
    }
}

/// An action for the event loop to carry out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    None,
    Run {
        what: String,
        args: Vec<String>,
        reload: bool,
    },
    /// Name a speaker: one `meeting label` per key, in order, stopping at
    /// the first that fails.
    Label {
        what: String,
        commands: Vec<Vec<String>>,
        keys: Vec<String>,
        name: String,
    },
    Ask(String),
    OpenSelected,
    Refresh,
}

/// Handle one key. Pure: anything that needs the outside world comes back
/// as an [`Action`].
pub fn handle_key(app: &mut WatchApp, key: KeyEvent) -> Action {
    if key.kind != KeyEventKind::Press {
        return Action::None;
    }
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        app.quit = true;
        return Action::None;
    }
    if app.modal != Modal::None {
        return handle_modal_key(app, key);
    }
    match app.screen {
        Screen::List => match key.code {
            KeyCode::Char('q') | KeyCode::Esc => {
                app.quit = true;
                Action::None
            }
            KeyCode::Up | KeyCode::Char('k') => {
                app.list_selected = app.list_selected.saturating_sub(1);
                Action::None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if app.list_selected + 1 < app.list_len() {
                    app.list_selected += 1;
                }
                Action::None
            }
            KeyCode::Enter => Action::OpenSelected,
            KeyCode::Char('r') => Action::Refresh,
            _ => Action::None,
        },
        Screen::Meeting => meeting_key(app, key),
    }
}

fn meeting_key(app: &mut WatchApp, key: KeyEvent) -> Action {
    let id = app.view().meeting_id.clone();
    match key.code {
        KeyCode::Char('q') => {
            app.quit = true;
            Action::None
        }
        KeyCode::Esc => {
            if app.ask.is_some() {
                app.ask = None;
            } else {
                app.stored = None;
                app.screen = Screen::List;
                return Action::Refresh;
            }
            Action::None
        }
        KeyCode::Up | KeyCode::Char('k') => {
            app.move_selection(-1);
            Action::None
        }
        KeyCode::Down | KeyCode::Char('j') => {
            app.move_selection(1);
            Action::None
        }
        KeyCode::PageUp => {
            app.move_selection(-10);
            Action::None
        }
        KeyCode::PageDown => {
            app.move_selection(10);
            Action::None
        }
        KeyCode::Home => {
            app.follow = false;
            if !app.view().lines().is_empty() {
                app.selected = Some(0);
            }
            Action::None
        }
        KeyCode::End => {
            app.follow = true;
            app.keep_selection_in_range();
            Action::None
        }
        KeyCode::Char('p') => {
            if app.stored.is_some() {
                app.message = Some("This meeting has ended".into());
                return Action::None;
            }
            let (what, cmd) = match app.live.state {
                ViewState::Recording => ("Pause", "pause"),
                ViewState::Paused => ("Resume", "resume"),
                _ => {
                    app.message = Some("No meeting is recording".into());
                    return Action::None;
                }
            };
            Action::Run {
                what: what.into(),
                args: vec!["meeting".into(), cmd.into()],
                reload: false,
            }
        }
        KeyCode::Char('s') => {
            if app.stored.is_none() && app.live.state.is_live() {
                app.modal = Modal::ConfirmStop;
            } else {
                app.message = Some("No meeting is recording".into());
            }
            Action::None
        }
        KeyCode::Char('r') | KeyCode::Char('n') | KeyCode::Char('a') | KeyCode::Char('l')
            if id.is_none() =>
        {
            app.message = Some("No meeting to act on".into());
            Action::None
        }
        KeyCode::Char('r') => {
            if app.selected_segment().is_none() {
                app.message = Some("Select a line first".into());
                return Action::None;
            }
            let mut choices: Vec<String> =
                app.view().speakers().into_iter().map(|c| c.name).collect();
            choices.push("New speaker…".into());
            app.modal = Modal::Reassign {
                choices,
                selected: 0,
                typing: None,
            };
            Action::None
        }
        KeyCode::Char('l') => {
            let chips = app.view().speakers();
            if chips.is_empty() {
                app.message = Some("No speakers yet".into());
                return Action::None;
            }
            // Start on the selected line's speaker.
            let selected = app
                .selected_segment()
                .and_then(|s| app.view().speaker_index(s))
                .unwrap_or(0);
            app.modal = Modal::Label {
                chips,
                selected,
                typing: None,
            };
            Action::None
        }
        KeyCode::Char('n') => {
            if app.selected_segment().is_none() {
                app.message = Some("Select a line first".into());
                return Action::None;
            }
            app.modal = Modal::Note(String::new());
            Action::None
        }
        KeyCode::Char('a') => {
            app.modal = Modal::Ask(String::new());
            Action::None
        }
        _ => Action::None,
    }
}

fn edit(buffer: &mut String, key: KeyEvent) {
    match key.code {
        KeyCode::Char(c) => buffer.push(c),
        KeyCode::Backspace => {
            buffer.pop();
        }
        _ => {}
    }
}

fn handle_modal_key(app: &mut WatchApp, key: KeyEvent) -> Action {
    let id = app.view().meeting_id.clone().unwrap_or_default();
    let segment = app.selected_segment().map(|s| s.id);
    let modal = std::mem::replace(&mut app.modal, Modal::None);
    match modal {
        Modal::None => Action::None,
        Modal::ConfirmStop => match key.code {
            KeyCode::Char('y') | KeyCode::Enter => Action::Run {
                what: "Stop".into(),
                args: vec!["meeting".into(), "stop".into()],
                reload: false,
            },
            _ => Action::None,
        },
        Modal::Label {
            chips,
            mut selected,
            typing,
        } => {
            if let Some(mut name) = typing {
                match key.code {
                    KeyCode::Esc => {}
                    KeyCode::Enter if !name.trim().is_empty() => {
                        return label(&id, &chips[selected], name.trim());
                    }
                    _ => {
                        edit(&mut name, key);
                        app.modal = Modal::Label {
                            chips,
                            selected,
                            typing: Some(name),
                        };
                    }
                }
                return Action::None;
            }
            match key.code {
                KeyCode::Esc => return Action::None,
                KeyCode::Up | KeyCode::Char('k') => selected = selected.saturating_sub(1),
                KeyCode::Down | KeyCode::Char('j') => {
                    selected = (selected + 1).min(chips.len() - 1)
                }
                KeyCode::Enter => {
                    app.modal = Modal::Label {
                        chips,
                        selected,
                        typing: Some(String::new()),
                    };
                    return Action::None;
                }
                _ => {}
            }
            app.modal = Modal::Label {
                chips,
                selected,
                typing: None,
            };
            Action::None
        }
        Modal::Reassign {
            choices,
            mut selected,
            typing,
        } => {
            if let Some(mut name) = typing {
                match key.code {
                    KeyCode::Esc => {}
                    KeyCode::Enter if !name.trim().is_empty() => {
                        return reassign(app, &id, segment, name.trim());
                    }
                    _ => {
                        edit(&mut name, key);
                        app.modal = Modal::Reassign {
                            choices,
                            selected,
                            typing: Some(name),
                        };
                    }
                }
                return Action::None;
            }
            match key.code {
                KeyCode::Esc => Action::None,
                KeyCode::Up | KeyCode::Char('k') => {
                    selected = selected.saturating_sub(1);
                    app.modal = Modal::Reassign {
                        choices,
                        selected,
                        typing: None,
                    };
                    Action::None
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    selected = (selected + 1).min(choices.len() - 1);
                    app.modal = Modal::Reassign {
                        choices,
                        selected,
                        typing: None,
                    };
                    Action::None
                }
                KeyCode::Enter if selected + 1 == choices.len() => {
                    app.modal = Modal::Reassign {
                        choices,
                        selected,
                        typing: Some(String::new()),
                    };
                    Action::None
                }
                KeyCode::Enter => {
                    let name = choices[selected].clone();
                    reassign(app, &id, segment, &name)
                }
                _ => {
                    app.modal = Modal::Reassign {
                        choices,
                        selected,
                        typing: None,
                    };
                    Action::None
                }
            }
        }
        Modal::Note(mut text) => match key.code {
            KeyCode::Esc => Action::None,
            KeyCode::Enter if !text.trim().is_empty() => match segment {
                Some(seg) => Action::Run {
                    what: "Note".into(),
                    args: vec![
                        "meeting".into(),
                        "annotate".into(),
                        id,
                        seg.to_string(),
                        text.trim().to_string(),
                    ],
                    reload: true,
                },
                None => Action::None,
            },
            _ => {
                edit(&mut text, key);
                app.modal = Modal::Note(text);
                Action::None
            }
        },
        Modal::Ask(mut text) => match key.code {
            KeyCode::Esc => Action::None,
            KeyCode::Enter if !text.trim().is_empty() => Action::Ask(text.trim().to_string()),
            _ => {
                edit(&mut text, key);
                app.modal = Modal::Ask(text);
                Action::None
            }
        },
    }
}

/// Name a speaker chip: one `meeting label` per diarizer ID behind it, as
/// the Quickshell view does, so a merged chip stays merged.
fn label(id: &str, chip: &SpeakerChip, name: &str) -> Action {
    Action::Label {
        what: format!("Name {}", chip.name),
        keys: chip.keys.clone(),
        name: name.to_string(),
        commands: chip
            .keys
            .iter()
            .map(|key| {
                vec![
                    "meeting".into(),
                    "label".into(),
                    id.to_string(),
                    key.clone(),
                    name.to_string(),
                ]
            })
            .collect(),
    }
}

fn reassign(app: &mut WatchApp, id: &str, segment: Option<u32>, name: &str) -> Action {
    let Some(seg) = segment else {
        app.message = Some("Select a line first".into());
        return Action::None;
    };
    Action::Run {
        what: "Reassign".into(),
        args: vec![
            "meeting".into(),
            "reassign".into(),
            id.to_string(),
            seg.to_string(),
            name.to_string(),
        ],
        reload: true,
    }
}

/// Apply a child process's result.
pub fn on_outcome(
    app: &mut WatchApp,
    outcome: Outcome,
    reload: &dyn Fn(&str) -> Option<MeetingView>,
) {
    match outcome {
        Outcome::Done {
            what,
            ok,
            message,
            reload: wants_reload,
            label,
        } => {
            if let (true, Some((keys, name))) = (ok, label.as_ref()) {
                app.live.apply_label(keys, name);
            }
            app.message = Some(if ok {
                format!("{what}: done")
            } else {
                format!("{what} failed: {message}")
            });
            // A live meeting gets the change back on the socket. One that
            // has ended does not, so read it again from storage.
            let ended = app.stored.is_some() || !app.live.state.is_live();
            if ok && wants_reload && ended {
                if let Some(id) = app.view().meeting_id.clone() {
                    if let Some(view) = reload(&id) {
                        app.stored = Some(view);
                        app.keep_selection_in_range();
                    }
                }
            }
        }
        Outcome::AskLine(line) => {
            let Some(pane) = app.ask.as_mut() else {
                return;
            };
            let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
                return;
            };
            match v["event"].as_str() {
                Some("start") => pane.agent_name = v["agent_name"].as_str().map(String::from),
                Some("delta") => pane.answer.push_str(v["text"].as_str().unwrap_or_default()),
                Some("answer") => {
                    if let Some(a) = v["answer"].as_str() {
                        pane.answer = a.to_string();
                    }
                    pane.done = true;
                }
                Some("error") => {
                    pane.error = Some(v["message"].as_str().unwrap_or("failed").to_string());
                    pane.done = true;
                }
                _ => {}
            }
        }
        Outcome::AskExited(ok) => {
            if let Some(pane) = app.ask.as_mut() {
                if !pane.done {
                    pane.done = true;
                    if !ok && pane.error.is_none() {
                        pane.error = Some("The assistant did not answer".into());
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

/// Draw the whole screen.
pub fn draw(f: &mut Frame, app: &mut WatchApp, now: DateTime<Utc>) {
    let area = f.area();
    match app.screen {
        Screen::List => draw_list(f, app, area, now),
        Screen::Meeting => draw_meeting(f, app, area, now),
    }
    match &app.modal {
        Modal::None => {}
        Modal::ConfirmStop => popup(
            f,
            area,
            "Stop meeting",
            Text::from("Stop the meeting? y to stop, any other key to keep recording."),
        ),
        Modal::Label {
            chips,
            selected,
            typing,
        } => {
            let mut lines: Vec<Line> = Vec::new();
            if let Some(name) = typing {
                lines.push(Line::from(format!(
                    "New name for {}:",
                    chips[*selected].name
                )));
                lines.push(Line::from(format!("> {name}_")));
            } else {
                for (i, c) in chips.iter().enumerate() {
                    let style = if i == *selected {
                        Style::default().add_modifier(Modifier::REVERSED)
                    } else {
                        Style::default()
                    };
                    lines.push(Line::styled(format!(" {} ", c.name), style));
                }
            }
            popup(f, area, "Name a speaker", Text::from(lines));
        }
        Modal::Reassign {
            choices,
            selected,
            typing,
        } => {
            let mut lines: Vec<Line> = Vec::new();
            if let Some(name) = typing {
                lines.push(Line::from("New speaker's name:"));
                lines.push(Line::from(format!("> {name}_")));
            } else {
                for (i, c) in choices.iter().enumerate() {
                    let style = if i == *selected {
                        Style::default().add_modifier(Modifier::REVERSED)
                    } else {
                        Style::default()
                    };
                    lines.push(Line::styled(format!(" {c} "), style));
                }
            }
            popup(f, area, "Move this line to", Text::from(lines));
        }
        Modal::Note(text) => popup(
            f,
            area,
            "Note on this line",
            Text::from(format!("> {text}_")),
        ),
        Modal::Ask(text) => popup(
            f,
            area,
            "Ask about this meeting",
            Text::from(format!("> {text}_")),
        ),
    }
}

fn popup(f: &mut Frame, area: Rect, title: &str, text: Text) {
    let width = area.width.saturating_sub(4).min(64);
    let height = (text.lines.len() as u16 + 2)
        .min(area.height.saturating_sub(2))
        .max(3);
    let rect = Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    };
    f.render_widget(Clear, rect);
    f.render_widget(
        Paragraph::new(text).wrap(Wrap { trim: false }).block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" {title} ")),
        ),
        rect,
    );
}

fn state_span(view: &MeetingView) -> Span<'static> {
    let (text, color) = match view.state {
        ViewState::Recording => ("● REC", Color::Red),
        ViewState::Paused => ("❚❚ Paused", Color::Yellow),
        ViewState::Finalizing => ("… Finalizing", Color::Yellow),
        ViewState::Summarizing => ("… Summarizing", Color::Cyan),
        ViewState::Ended => ("■ Ended", Color::Gray),
        ViewState::Idle => ("○ Idle", Color::Gray),
    };
    Span::styled(
        text,
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    )
}

fn draw_meeting(f: &mut Frame, app: &mut WatchApp, area: Rect, now: DateTime<Utc>) {
    let ask_height = if app.ask.is_some() { 8 } else { 0 };
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(ask_height),
            Constraint::Length(1),
        ])
        .split(area);

    // Header: state, title, elapsed, connection.
    let view = app.view();
    let mut header = vec![
        state_span(view),
        Span::raw("  "),
        Span::styled(
            view.title.clone().unwrap_or_else(|| "Meeting".into()),
            Style::default().add_modifier(Modifier::BOLD),
        ),
    ];
    if let Some(d) = view.elapsed(now) {
        header.push(Span::raw(format!("  {}", clock(d))));
    }
    if let Some(p) = view.progress.as_ref().filter(|p| p.running()) {
        let what = if p.operation == "import" {
            "Importing"
        } else {
            "Transcribing again"
        };
        header.push(Span::styled(
            format!("  {what} {:.0}%", p.fraction() * 100.0),
            Style::default().fg(Color::Cyan),
        ));
    }
    if let Some(l) = view.current_levels(now) {
        header.push(Span::raw("  "));
        header.extend(level_spans("mic", l.mic));
        if l.loopback.is_some() {
            header.push(Span::raw(" "));
            header.extend(level_spans("call", l.loopback));
        }
    }
    if view.playback.is_some() {
        header.push(Span::styled(
            "  ▶ playing",
            Style::default().fg(Color::Green),
        ));
    }
    if view.live && !view.connected {
        header.push(Span::styled(
            "  reconnecting…",
            Style::default().fg(Color::Red),
        ));
    }
    if !app.follow && view.state.is_live() {
        header.push(Span::styled(
            "  (paused scrolling: End to follow)",
            Style::default().fg(Color::DarkGray),
        ));
    }
    f.render_widget(Paragraph::new(Line::from(header)), rows[0]);

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(30), Constraint::Length(28)])
        .split(rows[1]);
    draw_transcript(f, app, cols[0], now);
    draw_side(f, app, cols[1]);

    if let Some(pane) = &app.ask {
        let who = pane
            .agent_name
            .clone()
            .unwrap_or_else(|| "Assistant".into());
        let mut lines = vec![Line::styled(
            format!("Q: {}", pane.question),
            Style::default().add_modifier(Modifier::BOLD),
        )];
        if let Some(e) = &pane.error {
            lines.push(Line::styled(e.clone(), Style::default().fg(Color::Red)));
        } else if pane.answer.is_empty() && !pane.done {
            lines.push(Line::styled("…", Style::default().fg(Color::DarkGray)));
        } else {
            lines.extend(pane.answer.lines().map(|l| Line::from(l.to_string())));
        }
        let title = if pane.done {
            format!(" {who} (Esc closes) ")
        } else {
            format!(" {who} is answering… ")
        };
        f.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .block(Block::default().borders(Borders::TOP).title(title)),
            rows[2],
        );
    }

    let help = if app.stored.is_some() {
        "↑↓ select  l name speaker  r move line  n note  a ask  Esc back  q quit"
    } else {
        "↑↓ select  End follow  p pause/resume  s stop  l name speaker  r move line  n note  a ask  Esc list  q quit"
    };
    let footer = match &app.message {
        Some(m) => Line::from(vec![
            Span::styled(m.clone(), Style::default().fg(Color::Yellow)),
            Span::raw("   "),
            Span::styled(help, Style::default().fg(Color::DarkGray)),
        ]),
        None => Line::styled(help, Style::default().fg(Color::DarkGray)),
    };
    f.render_widget(Paragraph::new(footer), rows[3]);
}

/// A five-cell meter for one track's level.
fn level_spans(label: &'static str, db: Option<f32>) -> Vec<Span<'static>> {
    let filled = (meter(db) * 5.0).round() as usize;
    vec![
        Span::styled(format!("{label} "), Style::default().fg(Color::DarkGray)),
        Span::styled("▮".repeat(filled), Style::default().fg(Color::Green)),
        Span::styled("▯".repeat(5 - filled), Style::default().fg(Color::DarkGray)),
    ]
}

/// One transcript line (plus its annotation) as text.
fn line_text(
    view: &MeetingView,
    seg: &TranscriptSegment,
    selected: bool,
    playing: bool,
) -> Text<'static> {
    let color = view
        .speaker_index(seg)
        .map(|i| PALETTE[i % PALETTE.len()])
        .unwrap_or(Color::Gray);
    let base = if selected {
        Style::default().add_modifier(Modifier::REVERSED)
    } else {
        Style::default()
    };
    let marks = LineMarks::of(seg);
    let mut first = vec![
        Span::styled(
            if playing { "▶ " } else { "" },
            base.fg(Color::Green).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("[{}] ", clock(Duration::from_millis(seg.start_ms))),
            base.fg(Color::DarkGray),
        ),
        Span::styled(
            seg.speaker_display(),
            base.fg(color).add_modifier(Modifier::BOLD),
        ),
    ];
    if let Some(from) = seg.original_speaker.as_deref().filter(|_| marks.moved) {
        first.push(Span::styled(
            format!(" (was {from})"),
            base.fg(Color::DarkGray),
        ));
    }
    let text_style = if marks.highlighted {
        base.fg(Color::Yellow)
    } else {
        base
    };
    first.push(Span::styled(format!(": {}", seg.text.trim()), text_style));
    if marks.edited {
        first.push(Span::styled(
            " (edited)",
            base.fg(Color::DarkGray).add_modifier(Modifier::ITALIC),
        ));
    }
    let mut lines = vec![Line::from(first)];
    if let Some(a) = &seg.annotation {
        lines.push(Line::styled(
            format!("    ✎ {}", crate::meeting::export::annotation_line(a)),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::ITALIC),
        ));
    }
    Text::from(lines)
}

fn draw_transcript(f: &mut Frame, app: &mut WatchApp, area: Rect, now: DateTime<Utc>) {
    let block = Block::default().borders(Borders::RIGHT);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let view = app.view();
    if view.lines().is_empty() {
        let msg = match view.state {
            ViewState::Idle => "No meeting is running. Esc lists recent meetings.",
            _ => "Listening… lines appear as each chunk is transcribed.",
        };
        f.render_widget(
            Paragraph::new(Line::styled(msg, Style::default().fg(Color::DarkGray)))
                .wrap(Wrap { trim: false }),
            inner,
        );
        return;
    }

    let width = inner.width.max(1);
    let playing = view.playing_line(now);
    let mut text = Text::default();
    let mut tops = Vec::with_capacity(view.lines().len());
    let mut row = 0usize;
    for (i, seg) in view.lines().iter().enumerate() {
        let t = line_text(view, seg, Some(i) == app.selected, playing == Some(seg.id));
        let height = Paragraph::new(t.clone())
            .wrap(Wrap { trim: false })
            .line_count(width);
        tops.push((row, height));
        row += height;
        text.extend(t);
    }
    let total = row;
    let visible = inner.height as usize;
    let mut scroll = app.scroll.min(total.saturating_sub(1));
    if app.follow {
        scroll = total.saturating_sub(visible);
    } else if let Some(sel) = app.selected {
        let (top, h) = tops[sel.min(tops.len() - 1)];
        if top < scroll {
            scroll = top;
        } else if top + h > scroll + visible {
            scroll = (top + h).saturating_sub(visible);
        }
    }
    app.scroll = scroll;
    f.render_widget(
        Paragraph::new(text)
            .wrap(Wrap { trim: false })
            .scroll((scroll.min(u16::MAX as usize) as u16, 0)),
        inner,
    );
}

fn draw_side(f: &mut Frame, app: &WatchApp, area: Rect) {
    let view = app.view();
    let mut lines = vec![Line::styled(
        "Speakers",
        Style::default().add_modifier(Modifier::BOLD),
    )];
    for (i, chip) in view.speakers().iter().enumerate() {
        lines.push(Line::from(vec![
            Span::styled("● ", Style::default().fg(PALETTE[i % PALETTE.len()])),
            Span::raw(format!("{} ({})", chip.name, chip.lines)),
        ]));
    }
    if let Some(s) = &view.summary {
        lines.push(Line::raw(""));
        lines.push(Line::styled(
            format!("Summary ({})", s.agent_name),
            Style::default().add_modifier(Modifier::BOLD),
        ));
        lines.push(Line::raw(s.summary.summary.clone()));
        if !s.summary.action_items.is_empty() {
            lines.push(Line::raw(format!(
                "{} action item(s)",
                s.summary.action_items.len()
            )));
        }
    } else if view.state == ViewState::Summarizing {
        lines.push(Line::raw(""));
        lines.push(Line::styled(
            "Writing the summary…",
            Style::default().fg(Color::Cyan),
        ));
    }
    if let Some(e) = &view.last_error {
        lines.push(Line::raw(""));
        lines.push(Line::styled(e.clone(), Style::default().fg(Color::Red)));
    }
    f.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }),
        Rect {
            x: area.x + 1,
            width: area.width.saturating_sub(1),
            ..area
        },
    );
}

fn draw_list(f: &mut Frame, app: &mut WatchApp, area: Rect, now: DateTime<Utc>) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(area);
    let header = if app.live.connected {
        "Meetings"
    } else {
        "Meetings (the daemon is not running)"
    };
    let mut spans = vec![Span::styled(
        header,
        Style::default().add_modifier(Modifier::BOLD),
    )];
    // A sound check shows its levels here, where no meeting is on screen.
    if let Some(l) = app.live.current_levels(now).filter(|l| l.soundcheck) {
        spans.push(Span::styled(
            "  Sound check  ",
            Style::default().fg(Color::Cyan),
        ));
        spans.extend(level_spans("mic", l.mic));
        if l.loopback.is_some() {
            spans.push(Span::raw(" "));
            spans.extend(level_spans("call", l.loopback));
        }
    }
    f.render_widget(Paragraph::new(Line::from(spans)), rows[0]);

    let mut items: Vec<ListItem> = Vec::new();
    if app.has_live_row() {
        items.push(ListItem::new(Line::from(vec![
            state_span(&app.live),
            Span::raw(format!(
                "  {}",
                app.live
                    .title
                    .clone()
                    .unwrap_or_else(|| "Live meeting".into())
            )),
        ])));
    }
    for e in &app.list {
        let m = &e.metadata;
        let duration = m
            .duration_secs
            .map(|d| clock(Duration::from_secs(d)))
            .unwrap_or_else(|| "--:--".into());
        let summary = if e.has_summary { "  summary" } else { "" };
        items.push(ListItem::new(Line::from(vec![
            Span::styled(
                m.started_at
                    .with_timezone(&chrono::Local)
                    .format("%Y-%m-%d %H:%M  ")
                    .to_string(),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(
                m.display_title(),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!(
                "  {duration}  {} speaker{}{summary}",
                e.speaker_count,
                if e.speaker_count == 1 { "" } else { "s" }
            )),
        ])));
    }
    if items.is_empty() {
        f.render_widget(
            Paragraph::new(Line::styled(
                "No meetings yet. Start one with: voxtype meeting start",
                Style::default().fg(Color::DarkGray),
            )),
            rows[1],
        );
    } else {
        let mut state = ListState::default().with_selected(Some(app.list_selected));
        f.render_stateful_widget(
            List::new(items).highlight_style(Style::default().add_modifier(Modifier::REVERSED)),
            rows[1],
            &mut state,
        );
    }
    let footer = match &app.message {
        Some(m) => Line::styled(m.clone(), Style::default().fg(Color::Yellow)),
        None => Line::styled(
            "↑↓ select  Enter open  r refresh  q quit",
            Style::default().fg(Color::DarkGray),
        ),
    };
    f.render_widget(Paragraph::new(footer), rows[2]);
}

// ---------------------------------------------------------------------------
// The loop
// ---------------------------------------------------------------------------

/// Run `commands` in order on a thread, stopping at the first failure, and
/// report the last one run.
fn run_commands(
    opts: &WatchOptions,
    what: String,
    commands: Vec<Vec<String>>,
    reload: bool,
    label: Option<(Vec<String>, String)>,
    tx: Sender<Outcome>,
) {
    let exe = opts.exe.clone();
    let global = opts.global_args.clone();
    std::thread::spawn(move || {
        let mut outcome = None;
        for args in commands {
            let mut cmd = Command::new(&exe);
            cmd.args(&global).args(&args);
            let result = run_one(cmd, what.clone(), reload, label.clone());
            let failed = matches!(result, Outcome::Done { ok: false, .. });
            outcome = Some(result);
            if failed {
                break;
            }
        }
        if let Some(outcome) = outcome {
            let _ = tx.send(outcome);
        }
    });
}

fn run_one(
    mut cmd: Command,
    what: String,
    reload: bool,
    label: Option<(Vec<String>, String)>,
) -> Outcome {
    match cmd.stdin(Stdio::null()).output() {
        Ok(o) => {
            let text = if o.status.success() {
                String::from_utf8_lossy(&o.stdout).into_owned()
            } else {
                String::from_utf8_lossy(&o.stderr).into_owned()
            };
            Outcome::Done {
                what,
                ok: o.status.success(),
                message: text
                    .lines()
                    .map(|l| l.trim_start_matches("Error: ").trim())
                    .find(|l| !l.is_empty())
                    .unwrap_or_default()
                    .to_string(),
                reload,
                label,
            }
        }
        Err(e) => Outcome::Done {
            what,
            ok: false,
            message: e.to_string(),
            reload: false,
            label: None,
        },
    }
}

fn run_ask(opts: &WatchOptions, meeting_id: String, question: String, tx: Sender<Outcome>) {
    let mut cmd = Command::new(&opts.exe);
    cmd.args(&opts.global_args)
        .args(["meeting", "ask", &meeting_id, &question, "--json"]);
    std::thread::spawn(move || {
        let child = cmd
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn();
        let mut child = match child {
            Ok(c) => c,
            Err(_) => {
                let _ = tx.send(Outcome::AskExited(false));
                return;
            }
        };
        if let Some(out) = child.stdout.take() {
            for line in std::io::BufReader::new(out).lines().map_while(Result::ok) {
                if tx.send(Outcome::AskLine(line)).is_err() {
                    break;
                }
            }
        }
        let ok = child.wait().map(|s| s.success()).unwrap_or(false);
        let _ = tx.send(Outcome::AskExited(ok));
    });
}

fn load_stored(meetings: &MeetingConfig, id: &str) -> Option<MeetingView> {
    crate::meeting::get_meeting(meetings, id)
        .ok()
        .map(|m| MeetingView::stored(&m))
}

fn load_list(meetings: &MeetingConfig) -> Vec<MeetingListEntry> {
    crate::meeting::list_meeting_entries(meetings, Some(50)).unwrap_or_default()
}

/// Run the screen until the user quits.
pub fn run(opts: WatchOptions) -> anyhow::Result<()> {
    let (feed_tx, feed_rx) = mpsc::channel::<Feed>();
    let feed = spawn_feed(opts.socket.clone(), feed_tx);
    let (out_tx, out_rx): (Sender<Outcome>, Receiver<Outcome>) = mpsc::channel();

    let mut app = WatchApp::new();
    app.list = load_list(&opts.meetings);
    let started = Instant::now();

    let mut terminal = super::enter_terminal()?;
    let result = (|| -> anyhow::Result<()> {
        loop {
            while let Ok(msg) = feed_rx.try_recv() {
                if app.on_feed(msg) {
                    feed.resync();
                }
            }
            // Give the daemon a moment to answer before showing the list.
            if app.starting && started.elapsed() > Duration::from_millis(800) {
                app.starting = false;
            }
            while let Ok(outcome) = out_rx.try_recv() {
                on_outcome(&mut app, outcome, &|id| load_stored(&opts.meetings, id));
            }
            terminal.draw(|f| draw(f, &mut app, Utc::now()))?;
            if app.quit {
                return Ok(());
            }
            if !event::poll(Duration::from_millis(100))? {
                continue;
            }
            let Event::Key(key) = event::read()? else {
                continue;
            };
            app.message = None;
            match handle_key(&mut app, key) {
                Action::None => {}
                Action::Run { what, args, reload } => {
                    app.message = Some(format!("{what}…"));
                    run_commands(&opts, what, vec![args], reload, None, out_tx.clone());
                }
                Action::Label {
                    what,
                    commands,
                    keys,
                    name,
                } => {
                    app.message = Some(format!("{what}…"));
                    let label = Some((keys, name));
                    run_commands(&opts, what, commands, true, label, out_tx.clone());
                }
                Action::Ask(question) => {
                    if let Some(id) = app.view().meeting_id.clone() {
                        app.ask = Some(AskPane {
                            question: question.clone(),
                            ..Default::default()
                        });
                        run_ask(&opts, id, question, out_tx.clone());
                    }
                }
                Action::OpenSelected => {
                    app.open_selected(&|id| load_stored(&opts.meetings, id));
                }
                Action::Refresh => {
                    app.list = load_list(&opts.meetings);
                    app.list_selected = app.list_selected.min(app.list_len().saturating_sub(1));
                }
            }
        }
    })();
    super::leave_terminal(&mut terminal)?;
    feed.stop();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meeting::data::{MeetingData, MeetingMetadata};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn seg(id: u32, secs: u64, speaker: &str, text: &str) -> serde_json::Value {
        serde_json::json!({
            "id": id, "start_ms": secs * 1000, "end_ms": secs * 1000 + 900, "text": text,
            "source": "loopback", "speaker_id": speaker, "chunk_id": 0
        })
    }

    fn hello(status: &str, segments: Vec<serde_json::Value>) -> Feed {
        Feed::Line(
            serde_json::json!({
                "event": "hello", "status": status, "meeting_id": "m1", "title": "Standup",
                "started_at": "2026-11-11T14:00:00Z", "chunk_count": 1, "segments": segments
            })
            .to_string(),
        )
    }

    fn live_app(n: u32) -> WatchApp {
        let mut app = WatchApp::new();
        app.on_feed(Feed::Connected);
        let segs = (0..n)
            .map(|i| {
                let who = if i % 2 == 0 { "You" } else { "Remote" };
                seg(i, i as u64 * 5, who, &format!("line number {i}"))
            })
            .collect();
        app.on_feed(hello("recording", segs));
        app
    }

    fn render(app: &mut WatchApp, w: u16, h: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        let now: DateTime<Utc> = "2026-11-11T14:02:05Z".parse().unwrap();
        terminal.draw(|f| draw(f, app, now)).unwrap();
        let buf = terminal.backend().buffer().clone();
        (0..h)
            .map(|y| {
                (0..w)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn a_live_meeting_opens_on_the_meeting_screen_and_follows() {
        let mut app = live_app(3);
        assert_eq!(app.screen, Screen::Meeting);
        assert_eq!(app.selected, Some(2));
        let screen = render(&mut app, 100, 12);
        assert!(screen.contains("● REC  Standup  02:05"), "{screen}");
        assert!(screen.contains("[00:00] You: line number 0"), "{screen}");
        assert!(screen.contains("[00:10] You: line number 2"), "{screen}");
        assert!(screen.contains("Speakers"), "{screen}");
        assert!(
            screen.contains("You (2)") && screen.contains("Remote (1)"),
            "{screen}"
        );
        assert!(screen.contains("p pause/resume"), "{screen}");
    }

    #[test]
    fn new_lines_scroll_into_view_until_the_user_scrolls_up() {
        let mut app = live_app(30);
        let screen = render(&mut app, 80, 10);
        assert!(screen.contains("line number 29"), "{screen}");
        // Scroll up: stop following.
        for _ in 0..20 {
            handle_key(&mut app, key(KeyCode::Up));
        }
        assert!(!app.follow);
        assert_eq!(app.selected, Some(9));
        app.on_feed(Feed::Line(
            serde_json::json!({"event":"segments","chunk_id":9,"segments":[seg(30, 200, "You", "brand new")]})
                .to_string(),
        ));
        let screen = render(&mut app, 80, 10);
        assert!(screen.contains("line number 9"), "{screen}");
        assert!(!screen.contains("brand new"), "{screen}");
        assert!(screen.contains("End to follow"), "{screen}");
        // End resumes following.
        handle_key(&mut app, key(KeyCode::End));
        let screen = render(&mut app, 80, 10);
        assert!(screen.contains("brand new"), "{screen}");
        assert_eq!(app.selected, Some(30));
    }

    #[test]
    fn states_show_in_the_header() {
        let mut app = live_app(1);
        app.on_feed(Feed::Line(r#"{"event":"status","status":"paused"}"#.into()));
        assert!(render(&mut app, 90, 8).contains("❚❚ Paused"));
        app.on_feed(Feed::Disconnected);
        assert!(render(&mut app, 90, 8).contains("reconnecting…"));
        app.on_feed(hello("recording", vec![seg(0, 0, "You", "line number 0")]));
        app.on_feed(Feed::Line(r#"{"event":"status","status":"idle"}"#.into()));
        assert!(render(&mut app, 90, 8).contains("■ Ended"));
        app.on_feed(Feed::Line(
            r#"{"event":"status","status":"summarizing","meeting_id":"m1"}"#.into(),
        ));
        let screen = render(&mut app, 90, 8);
        assert!(screen.contains("… Summarizing"), "{screen}");
        assert!(screen.contains("Writing the summary"), "{screen}");
        app.on_feed(Feed::Line(
            serde_json::json!({"event":"summary_ready","meeting_id":"m1","agent":"claude",
                "agent_name":"Claude","summary":{"summary":"Shipped it.","generated_at":"2026-11-11T15:00:00Z",
                "action_items":[{"description":"tag"}]}})
            .to_string(),
        ));
        let screen = render(&mut app, 90, 10);
        assert!(screen.contains("Summary (Claude)"), "{screen}");
        assert!(screen.contains("Shipped it."), "{screen}");
        assert!(screen.contains("1 action item(s)"), "{screen}");
    }

    #[test]
    fn pause_resume_and_stop_run_the_meeting_commands() {
        let mut app = live_app(1);
        assert_eq!(
            handle_key(&mut app, key(KeyCode::Char('p'))),
            Action::Run {
                what: "Pause".into(),
                args: vec!["meeting".into(), "pause".into()],
                reload: false
            }
        );
        app.on_feed(Feed::Line(r#"{"event":"status","status":"paused"}"#.into()));
        assert!(matches!(
            handle_key(&mut app, key(KeyCode::Char('p'))),
            Action::Run { ref args, .. } if args[1] == "resume"
        ));
        // Stop asks first; anything but y keeps recording.
        assert_eq!(handle_key(&mut app, key(KeyCode::Char('s'))), Action::None);
        assert_eq!(app.modal, Modal::ConfirmStop);
        assert!(render(&mut app, 90, 12).contains("Stop the meeting?"));
        assert_eq!(handle_key(&mut app, key(KeyCode::Char('n'))), Action::None);
        assert_eq!(app.modal, Modal::None);
        handle_key(&mut app, key(KeyCode::Char('s')));
        assert!(matches!(
            handle_key(&mut app, key(KeyCode::Char('y'))),
            Action::Run { ref args, .. } if args[1] == "stop"
        ));
    }

    #[test]
    fn reassign_picks_a_speaker_or_types_a_new_one() {
        let mut app = live_app(3);
        handle_key(&mut app, key(KeyCode::Up)); // line 1 (Remote)
        handle_key(&mut app, key(KeyCode::Char('r')));
        let screen = render(&mut app, 90, 14);
        assert!(screen.contains("Move this line to"), "{screen}");
        assert!(screen.contains("New speaker…"), "{screen}");
        // Choices: You, Remote, New speaker…
        assert_eq!(
            handle_key(&mut app, key(KeyCode::Enter)),
            Action::Run {
                what: "Reassign".into(),
                args: ["meeting", "reassign", "m1", "1", "You"]
                    .map(String::from)
                    .to_vec(),
                reload: true
            }
        );
        handle_key(&mut app, key(KeyCode::Char('r')));
        handle_key(&mut app, key(KeyCode::Down));
        handle_key(&mut app, key(KeyCode::Down));
        handle_key(&mut app, key(KeyCode::Enter));
        for c in "Casey".chars() {
            handle_key(&mut app, key(KeyCode::Char(c)));
        }
        assert!(render(&mut app, 90, 14).contains("> Casey_"));
        assert!(matches!(
            handle_key(&mut app, key(KeyCode::Enter)),
            Action::Run { ref args, .. } if args[4] == "Casey" && args[3] == "1"
        ));
    }

    #[test]
    fn notes_and_asks_take_a_line_of_text() {
        let mut app = live_app(2);
        handle_key(&mut app, key(KeyCode::Char('n')));
        for c in "check".chars() {
            handle_key(&mut app, key(KeyCode::Char(c)));
        }
        assert!(matches!(
            handle_key(&mut app, key(KeyCode::Enter)),
            Action::Run { ref args, .. } if args == &["meeting", "annotate", "m1", "1", "check"]
        ));
        handle_key(&mut app, key(KeyCode::Char('a')));
        for c in "what now".chars() {
            handle_key(&mut app, key(KeyCode::Char(c)));
        }
        assert_eq!(
            handle_key(&mut app, key(KeyCode::Enter)),
            Action::Ask("what now".into())
        );
    }

    #[test]
    fn the_ask_pane_streams_the_answer() {
        let mut app = live_app(1);
        app.ask = Some(AskPane {
            question: "What did Steve say?".into(),
            ..Default::default()
        });
        let none = |_: &str| None;
        for line in [
            r#"{"event":"start","agent":"claude","agent_name":"Claude"}"#,
            r#"{"event":"delta","text":"Steve said "}"#,
        ] {
            on_outcome(&mut app, Outcome::AskLine(line.into()), &none);
        }
        let screen = render(&mut app, 90, 16);
        assert!(screen.contains("Claude is answering…"), "{screen}");
        assert!(screen.contains("Q: What did Steve say?"), "{screen}");
        assert!(screen.contains("Steve said"), "{screen}");
        on_outcome(
            &mut app,
            Outcome::AskLine(r#"{"event":"answer","answer":"Steve said ship it [00:41]."}"#.into()),
            &none,
        );
        on_outcome(&mut app, Outcome::AskExited(true), &none);
        let screen = render(&mut app, 90, 16);
        assert!(screen.contains("Claude (Esc closes)"), "{screen}");
        assert!(screen.contains("ship it [00:41]"), "{screen}");

        // A refusal shows its message.
        app.ask = Some(AskPane::default());
        on_outcome(
            &mut app,
            Outcome::AskLine(r#"{"event":"error","code":"consent_required","message":"Allow it with: voxtype meeting assistant allow claude"}"#.into()),
            &none,
        );
        assert!(render(&mut app, 90, 16).contains("voxtype meeting assistant allow claude"));
    }

    fn entry(title: &str, has_summary: bool, speakers: u32) -> MeetingListEntry {
        let mut m = MeetingMetadata::new(Some(title.into()));
        m.duration_secs = Some(125);
        MeetingListEntry {
            metadata: m,
            has_summary,
            speaker_count: speakers,
        }
    }

    #[test]
    fn with_no_meeting_the_list_opens_a_stored_meeting_as_ended() {
        let mut app = WatchApp::new();
        app.on_feed(Feed::Connected);
        app.on_feed(Feed::Line(
            r#"{"event":"hello","status":"idle","chunk_count":0,"segments":[]}"#.into(),
        ));
        assert_eq!(app.screen, Screen::List);
        app.list = vec![entry("Retro", true, 3), entry("1:1", false, 1)];
        let screen = render(&mut app, 100, 8);
        assert!(
            screen.contains("Retro  02:05  3 speakers  summary"),
            "{screen}"
        );
        assert!(screen.contains("1:1  02:05  1 speaker"), "{screen}");

        handle_key(&mut app, key(KeyCode::Down));
        let mut data = MeetingData::new(Some("1:1".into()));
        data.add_segment(TranscriptSegment::new(0, 0, 900, "hello".into(), 0));
        let stored = MeetingView::stored(&data);
        app.open_selected(&|_| Some(stored.clone()));
        assert_eq!(app.screen, Screen::Meeting);
        let screen = render(&mut app, 100, 8);
        assert!(screen.contains("■ Ended  1:1"), "{screen}");
        assert!(screen.contains("Unknown: hello"), "{screen}");
        assert!(screen.contains("Esc back"), "{screen}");
        // Pause makes no sense here.
        assert_eq!(handle_key(&mut app, key(KeyCode::Char('p'))), Action::None);
        // Esc goes back to the list.
        assert_eq!(handle_key(&mut app, key(KeyCode::Esc)), Action::Refresh);
        assert_eq!(app.screen, Screen::List);
        assert!(app.stored.is_none());
    }

    #[test]
    fn a_live_meeting_is_the_first_row_of_the_list() {
        let mut app = live_app(1);
        handle_key(&mut app, key(KeyCode::Esc));
        assert_eq!(app.screen, Screen::List);
        app.list = vec![entry("Retro", false, 2)];
        let screen = render(&mut app, 100, 6);
        assert!(screen.contains("● REC  Standup"), "{screen}");
        app.list_selected = 0;
        app.open_selected(&|_| None);
        assert_eq!(app.screen, Screen::Meeting);
        assert!(app.stored.is_none());
        assert!(app.follow);
    }

    #[test]
    fn an_edit_to_an_ended_meeting_reloads_it_from_storage() {
        let mut app = live_app(2);
        app.on_feed(Feed::Line(r#"{"event":"status","status":"idle"}"#.into()));
        let mut data = MeetingData::new(Some("Standup".into()));
        data.add_segment(TranscriptSegment::new(0, 0, 900, "edited".into(), 0));
        let reloaded = MeetingView::stored(&data);
        on_outcome(
            &mut app,
            Outcome::Done {
                what: "Reassign".into(),
                ok: true,
                message: String::new(),
                reload: true,
                label: None,
            },
            &|id| {
                assert_eq!(id, "m1");
                Some(reloaded.clone())
            },
        );
        assert!(render(&mut app, 90, 8).contains("edited"));
        assert_eq!(app.message.as_deref(), Some("Reassign: done"));
    }

    #[test]
    fn annotations_show_under_their_line() {
        let mut app = WatchApp::new();
        let mut s = seg(0, 1, "You", "the number is 40k");
        s["annotation"] = serde_json::json!({"text": "check this number", "highlight": false});
        app.on_feed(hello("recording", vec![s]));
        let screen = render(&mut app, 90, 8);
        assert!(screen.contains("✎ Note: check this number"), "{screen}");
    }

    #[test]
    fn a_meeting_starting_while_watching_asks_for_a_resync() {
        let mut app = live_app(1);
        app.on_feed(Feed::Line(r#"{"event":"status","status":"idle"}"#.into()));
        assert!(app.on_feed(Feed::Line(
            r#"{"event":"status","status":"recording"}"#.into()
        )));
    }

    #[test]
    fn l_names_a_speaker_chip_with_one_label_per_key() {
        let mut app = WatchApp::new();
        let mut a = seg(0, 1, "Remote 1", "x");
        a["speaker_label"] = "Dana".into();
        let mut b = seg(1, 2, "Remote 2", "y");
        b["speaker_label"] = "Dana".into();
        app.on_feed(hello("recording", vec![seg(2, 0, "You", "hi"), a, b]));
        // The selected (newest) line is Dana's, so the picker starts there.
        handle_key(&mut app, key(KeyCode::Char('l')));
        let screen = render(&mut app, 90, 14);
        assert!(screen.contains("Name a speaker"), "{screen}");
        assert!(matches!(app.modal, Modal::Label { selected: 1, .. }));
        handle_key(&mut app, key(KeyCode::Enter));
        for c in "Dana Lee".chars() {
            handle_key(&mut app, key(KeyCode::Char(c)));
        }
        assert!(render(&mut app, 90, 14).contains("New name for Dana:"));
        let action = handle_key(&mut app, key(KeyCode::Enter));
        let Action::Label {
            commands,
            keys,
            name,
            ..
        } = action
        else {
            panic!("expected a label action, got {action:?}");
        };
        assert_eq!(
            commands,
            vec![
                ["meeting", "label", "m1", "Remote 1", "Dana Lee"]
                    .map(String::from)
                    .to_vec(),
                ["meeting", "label", "m1", "Remote 2", "Dana Lee"]
                    .map(String::from)
                    .to_vec(),
            ]
        );
        // The daemon does not announce labels; success applies it here.
        let none = |_: &str| None;
        on_outcome(
            &mut app,
            Outcome::Done {
                what: "Name Dana".into(),
                ok: true,
                message: String::new(),
                reload: true,
                label: Some((keys, name)),
            },
            &none,
        );
        let screen = render(&mut app, 100, 10);
        assert!(screen.contains("Dana Lee (2)"), "{screen}");
        assert!(screen.contains("l name speaker"), "{screen}");
    }

    #[test]
    fn lines_show_edits_moves_playback_and_levels() {
        let mut app = WatchApp::new();
        let mut edited = seg(0, 1, "You", "We ship Friday.");
        edited["original_text"] = "we ship fry day".into();
        let mut moved = seg(1, 3, "Remote 2", "moved line");
        moved["original_speaker"] = "Remote 1".into();
        app.on_feed(hello("recording", vec![edited, moved]));
        app.on_feed(Feed::Line(
            r#"{"event":"levels","meeting_id":"m1","t_ms":1,"mic":-30.0,"loopback":-60.0}"#.into(),
        ));
        app.on_feed(Feed::Line(
            r#"{"event":"playback","meeting_id":"m1","track":"mixed","state":"playing","from_ms":1000}"#
                .into(),
        ));
        // Draw at the moment the events arrived.
        let now = app.live.levels_at.unwrap();
        let mut terminal = Terminal::new(TestBackend::new(110, 8)).unwrap();
        terminal.draw(|f| draw(f, &mut app, now)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let screen: String = (0..8)
            .map(|y| {
                (0..110)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
                    + "\n"
            })
            .collect();
        assert!(screen.contains("We ship Friday. (edited)"), "{screen}");
        assert!(
            screen.contains("Remote 2 (was Remote 1): moved line"),
            "{screen}"
        );
        assert!(screen.contains("▶ [00:01] You"), "{screen}");
        assert!(screen.contains("mic ▮▮▮▯▯ call ▯▯▯▯▯"), "{screen}");
        assert!(screen.contains("▶ playing"), "{screen}");
    }

    #[test]
    fn a_deleted_line_leaves_the_screen() {
        let mut app = live_app(3);
        app.on_feed(Feed::Line(
            r#"{"event":"segment_deleted","meeting_id":"m1","segment_id":1}"#.into(),
        ));
        let screen = render(&mut app, 90, 10);
        assert!(!screen.contains("line number 1"), "{screen}");
        assert!(screen.contains("line number 2"), "{screen}");
        assert_eq!(app.selected, Some(1));
    }
}
