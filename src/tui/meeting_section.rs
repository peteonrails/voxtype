//! Meeting mode settings: every `meeting.*` key in the config schema.
//!
//! The page is a [`SchemaForm`] over the schema's meeting keys, so a key
//! added to the schema shows up here without touching this file, with the
//! labels and help the Quickshell Settings panel shows too. The calendar
//! link is a secret: it is set with `voxtype meeting calendar set-ics` and
//! shown only as set or not set.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::Rect,
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use super::app::{Action, App};
use super::common::FeedbackLevel;
use super::config_editor::{ConfigEditor, EditorError};
use super::schema_form::{FormOptions, SchemaForm, SecretRow};
use crate::config::schema::{self, KeySpec};

pub struct MeetingState {
    pub form: SchemaForm,
}

/// The schema's meeting keys, in schema order.
pub fn meeting_keys() -> Vec<&'static KeySpec> {
    schema::scalar_keys()
        .filter(|k| k.key.starts_with("meeting."))
        .collect()
}

/// Heading for each `[meeting.*]` table.
fn group_title(table: &str) -> String {
    match table {
        "meeting" => "General".into(),
        "meeting.audio" => "Audio".into(),
        "meeting.diarization" => "Speakers".into(),
        "meeting.calendar" => "Calendar".into(),
        "meeting.summary" => "Summary".into(),
        "meeting.actions" => "Action items".into(),
        "meeting.share" => "Sharing".into(),
        "meeting.detect" => "Call detection".into(),
        "meeting.assistant" => "Assistant".into(),
        other => {
            let tail = other.rsplit('.').next().unwrap_or(other).replace('_', " ");
            let mut c = tail.chars();
            match c.next() {
                Some(first) => first.to_uppercase().chain(c).collect(),
                None => String::new(),
            }
        }
    }
}

/// The help the page's first hand-written fields had, kept under their keys.
fn extra_help(key: &str) -> Vec<&'static str> {
    match key {
        "meeting.enabled" => vec![
            "Long-form recording mode. Voxtype chunks audio into segments, transcribes \
             each, and stitches a continuous transcript with timestamps.",
            "Persists segments to ~/.local/share/voxtype/meetings/ so a crash doesn't \
             lose your transcript.",
        ],
        "meeting.diarization.enabled" => vec![
            "Tags each segment with a speaker label (Speaker 1, Speaker 2, …) so the \
             transcript reads like dialogue.",
            "The ml and hybrid backends use an ONNX speaker-embedding model \
             (ECAPA-TDNN) plus clustering.",
        ],
        _ => Vec::new(),
    }
}

fn options() -> FormOptions {
    FormOptions {
        title: "Meeting Mode",
        group_title,
        master: Some("meeting.enabled"),
        extra_help,
    }
}

/// The calendar link's entry, from the store it lives in.
fn calendar_secret() -> Vec<SecretRow> {
    use crate::meeting::calendar::source::{schema_entry, Store};
    SecretRow::from_schema_entry(&schema_entry(&Store::new(Store::default_dir())))
        .into_iter()
        .collect()
}

impl MeetingState {
    pub fn load() -> Result<Self, EditorError> {
        let ed = ConfigEditor::load()?;
        Ok(Self::from_editor(&ed, calendar_secret()))
    }

    pub fn from_editor(ed: &ConfigEditor, secrets: Vec<SecretRow>) -> Self {
        Self {
            form: SchemaForm::new(options(), meeting_keys(), secrets, ed),
        }
    }

    pub fn save(&mut self) -> Action {
        match ConfigEditor::load() {
            Ok(mut ed) => self.form.save_with_editor(&mut ed),
            Err(e) => self.form.feedback = Some((FeedbackLevel::Err, format!("load: {e}"))),
        }
        Action::None
    }

    pub fn reset(&mut self) {
        match ConfigEditor::load() {
            Ok(ed) => self.form.revert(&ed),
            Err(e) => self.form.feedback = Some((FeedbackLevel::Err, format!("load: {e}"))),
        }
    }
}

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    match &app.meeting {
        Some(s) => s.form.render(f, area),
        None => {
            let block = Block::default().borders(Borders::ALL).title("Meeting");
            let inner = block.inner(area);
            f.render_widget(block, area);
            f.render_widget(
                Paragraph::new("Failed to load config.").wrap(Wrap { trim: true }),
                inner,
            );
        }
    }
}

pub fn handle_key(app: &mut App, key: KeyEvent) -> Action {
    let Some(state) = app.meeting.as_mut() else {
        return Action::None;
    };
    // The form takes navigation, changes and typing; save and revert are
    // the section's.
    if state.form.handle_key(key) {
        return Action::None;
    }
    match key.code {
        KeyCode::Char('s') => state.save(),
        KeyCode::Char('r') => {
            state.reset();
            Action::None
        }
        _ => Action::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::schema_form::{Pending, Row};
    use crossterm::event::{KeyEventKind, KeyEventState, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    fn secret(set: bool) -> Vec<SecretRow> {
        vec![SecretRow {
            key: "meeting.calendar.ics_link".into(),
            label: "Calendar (iCal link)".into(),
            description: "Your calendar's private iCal link.".into(),
            help: "Set it with: voxtype meeting calendar set-ics".into(),
            set,
        }]
    }

    /// The whole page, drawn with ratatui's test backend, as text.
    pub(crate) fn screen(state: &MeetingState, w: u16, h: u16) -> String {
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(w, h)).unwrap();
        terminal.draw(|f| state.form.render(f, f.area())).unwrap();
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

    /// Every row's label and value, as the settings panel lists them.
    fn rows_text(state: &MeetingState) -> String {
        state
            .form
            .row_specs()
            .iter()
            .map(|r| {
                if r.heading {
                    format!("[{}]", r.label)
                } else {
                    format!("  {:<32} {}", r.label, r.value)
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn state_for(source: &str) -> (tempfile::TempDir, std::path::PathBuf, MeetingState) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, source).unwrap();
        let ed = ConfigEditor::load_from_path(path.clone()).unwrap();
        let state = MeetingState::from_editor(&ed, secret(true));
        (dir, path, state)
    }

    fn save(state: &mut MeetingState, path: &std::path::Path) {
        let mut ed = ConfigEditor::load_from_path(path.to_path_buf()).unwrap();
        state.form.save_with_editor(&mut ed);
    }

    #[test]
    fn every_meeting_schema_key_is_on_the_page() {
        let (_dir, _path, state) = state_for("");
        let on_page: Vec<&str> = state
            .form
            .rows()
            .iter()
            .filter_map(|r| match r {
                Row::Key(k) => Some(k.key),
                _ => None,
            })
            .collect();
        let keys = meeting_keys();
        assert!(keys.len() > 40, "{} meeting keys", keys.len());
        for k in &keys {
            assert!(on_page.contains(&k.key), "{} is not on the page", k.key);
        }
        assert_eq!(on_page.len(), keys.len(), "no key twice");
        // And every one is drawn, under its schema label.
        let text = rows_text(&state);
        for k in keys {
            assert!(text.contains(k.label), "{} ({}) not drawn", k.label, k.key);
        }
    }

    #[test]
    fn keys_group_by_table_in_schema_order_with_the_secret_above_its_table() {
        let (_dir, _path, state) = state_for("");
        let headings: Vec<String> = state
            .form
            .rows()
            .iter()
            .filter_map(|r| match r {
                Row::Heading(h) => Some(h.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(
            headings,
            [
                "General",
                "Audio",
                "Speakers",
                "Calendar",
                "Summary",
                "Action items",
                "Sharing",
                "Call detection",
                "Assistant"
            ]
        );
        // Inside a group, schema order; the calendar link first in Calendar.
        let rows = state.form.rows();
        let at = |pred: &dyn Fn(&Row) -> bool| rows.iter().position(pred).unwrap();
        let link = at(&|r| matches!(r, Row::Secret(_)));
        let command = at(&|r| matches!(r, Row::Key(k) if k.key == "meeting.calendar.command"));
        assert_eq!(link + 1, command);
        assert!(matches!(&rows[link - 1], Row::Heading(h) if h == "Calendar"));
        let order: Vec<&str> = rows
            .iter()
            .filter_map(|r| match r {
                Row::Key(k) if k.table == "meeting.audio" => Some(k.key),
                _ => None,
            })
            .collect();
        let schema_order: Vec<&str> = meeting_keys()
            .into_iter()
            .filter(|k| k.table == "meeting.audio")
            .map(|k| k.key)
            .collect();
        assert_eq!(order, schema_order);
    }

    #[test]
    fn a_bool_toggles_and_round_trips_through_the_config_editor() {
        let (_dir, path, mut state) =
            state_for("# my meetings\n[meeting]\nenabled = false # off for now\n");
        assert!(state.form.focus_key("meeting.enabled"));
        state.form.handle_key(key(KeyCode::Right));
        assert!(state.form.is_dirty());
        save(&mut state, &path);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("# my meetings"), "comments kept: {text}");
        let doc: toml::Value = text.parse().unwrap();
        assert_eq!(doc["meeting"]["enabled"].as_bool(), Some(true));
        assert!(!state.form.is_dirty());
        // A live key and a restart key say which they are.
        let (level, msg) = state.form.feedback.clone().unwrap();
        assert!(matches!(level, FeedbackLevel::Warn), "{msg}");
        assert!(
            msg.contains("Needs a daemon restart: Meeting mode"),
            "{msg}"
        );
    }

    #[test]
    fn an_enum_cycles_through_its_choices_and_round_trips() {
        let (_dir, path, mut state) = state_for("");
        assert!(state.form.focus_key("meeting.view"));
        // auto, quickshell, gtk4, terminal, none
        state.form.handle_key(key(KeyCode::Right));
        state.form.handle_key(key(KeyCode::Right));
        assert!(rows_text(&state).contains("gtk4 *"));
        save(&mut state, &path);
        let doc: toml::Value = std::fs::read_to_string(&path).unwrap().parse().unwrap();
        assert_eq!(doc["meeting"]["view"].as_str(), Some("gtk4"));
        // meeting.view needs no restart.
        let (_, msg) = state.form.feedback.clone().unwrap();
        assert_eq!(msg, "Saved; takes effect now");
        // Left wraps from the first choice to the last.
        assert!(state.form.focus_key("meeting.audio.echo_cancel"));
        state.form.handle_key(key(KeyCode::Left));
        assert!(matches!(
            state.form.pending().get("meeting.audio.echo_cancel"),
            Some(Pending::Set(crate::config::schema::TypedValue::Str(s))) if s == "disabled"
        ));
    }

    #[test]
    fn a_string_is_typed_and_round_trips_and_empty_restores_the_default() {
        let (_dir, path, mut state) = state_for("[meeting]\nmy_name = \"Pat\"\n");
        assert!(state.form.focus_key("meeting.my_name"));
        state.form.handle_key(key(KeyCode::Enter));
        assert!(state.form.editing.is_some());
        for _ in 0..3 {
            state.form.handle_key(key(KeyCode::Backspace));
        }
        for c in "Dana".chars() {
            state.form.handle_key(key(KeyCode::Char(c)));
        }
        state.form.handle_key(key(KeyCode::Enter));
        save(&mut state, &path);
        let doc: toml::Value = std::fs::read_to_string(&path).unwrap().parse().unwrap();
        assert_eq!(doc["meeting"]["my_name"].as_str(), Some("Dana"));

        // Clearing it removes the key.
        state.form.handle_key(key(KeyCode::Enter));
        state
            .form
            .handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        state.form.handle_key(key(KeyCode::Enter));
        save(&mut state, &path);
        let doc: toml::Value = std::fs::read_to_string(&path).unwrap().parse().unwrap();
        assert!(doc["meeting"].get("my_name").is_none());
    }

    #[test]
    fn a_number_steps_within_its_bounds_types_and_round_trips() {
        let (_dir, path, mut state) = state_for("[meeting]\nchunk_duration_secs = 598\n");
        assert!(state.form.focus_key("meeting.chunk_duration_secs"));
        for _ in 0..5 {
            state.form.handle_key(key(KeyCode::Right));
        }
        assert!(
            rows_text(&state).contains("600 *"),
            "clamped at the maximum"
        );
        save(&mut state, &path);
        let doc: toml::Value = std::fs::read_to_string(&path).unwrap().parse().unwrap();
        assert_eq!(
            doc["meeting"]["chunk_duration_secs"].as_integer(),
            Some(600)
        );

        // Typed: out of range is refused and nothing is pending.
        state.form.handle_key(key(KeyCode::Enter));
        state
            .form
            .handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        for c in "2".chars() {
            state.form.handle_key(key(KeyCode::Char(c)));
        }
        state.form.handle_key(key(KeyCode::Enter));
        assert!(!state.form.is_dirty());
        let (_, msg) = state.form.feedback.clone().unwrap();
        assert!(msg.contains("expected 5 to 600"), "{msg}");

        // A float steps too.
        assert!(state.form.focus_key("meeting.view_opacity"));
        state.form.handle_key(key(KeyCode::Left));
        save(&mut state, &path);
        let doc: toml::Value = std::fs::read_to_string(&path).unwrap().parse().unwrap();
        assert_eq!(doc["meeting"]["view_opacity"].as_float(), Some(0.75));
    }

    #[test]
    fn a_rejected_file_is_left_alone_and_the_change_stays_pending() {
        // Hand-written junk the loader refuses: the editor validates the
        // whole file before replacing it.
        let (_dir, path, mut state) = state_for("[meeting]\nenabled = \"sometimes\"\n");
        assert!(state.form.focus_key("meeting.retain_audio"));
        state.form.handle_key(key(KeyCode::Right));
        save(&mut state, &path);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "[meeting]\nenabled = \"sometimes\"\n"
        );
        assert!(state.form.is_dirty());
        let (level, msg) = state.form.feedback.clone().unwrap();
        assert!(matches!(level, FeedbackLevel::Err), "{msg}");
    }

    #[test]
    fn the_secret_is_never_rendered() {
        // A link in the store and an API key in the file: neither reaches
        // the screen, only whether each is set.
        let link = "https://calendar.example.com/private-abc123/basic.ics";
        let (_dir, _path, mut state) =
            state_for("[meeting.summary]\nremote_api_key = \"sk-live-SECRET42\"\n");
        let text = rows_text(&state);
        assert!(
            text.contains("Calendar (iCal link)             set"),
            "{text}"
        );
        assert!(
            text.contains("Remote summary API key           set"),
            "{text}"
        );
        for focus in ["meeting.summary.remote_api_key", "meeting.calendar.command"] {
            state.form.focus_key(focus);
            let page = screen(&state, 160, 60);
            assert!(!page.contains("SECRET42"), "{page}");
            assert!(!page.contains(link), "{page}");
        }
        // Editing the key starts empty rather than showing it.
        state.form.focus_key("meeting.summary.remote_api_key");
        state.form.handle_key(key(KeyCode::Enter));
        assert_eq!(state.form.editing.as_ref().unwrap().buffer(), "");
        assert!(!screen(&state, 160, 60).contains("SECRET42"));
        // The secret row takes no edits.
        state.form.editing = None;
        state.form.move_cursor(0);
        let secret_at = state
            .form
            .rows()
            .iter()
            .position(|r| matches!(r, Row::Secret(_)))
            .unwrap();
        while !matches!(state.form.focused(), Some(Row::Secret(_))) {
            state.form.move_cursor(1);
        }
        assert!(secret_at > 0);
        state.form.handle_key(key(KeyCode::Enter));
        state.form.handle_key(key(KeyCode::Right));
        assert!(!state.form.is_dirty());
        assert!(screen(&state, 160, 60).contains("voxtype meeting calendar set-ics"));
        // Not set reads as such.
        let (_d, _p, unset) = {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("config.toml");
            let ed = ConfigEditor::load_from_path(path.clone()).unwrap();
            (dir, path, MeetingState::from_editor(&ed, secret(false)))
        };
        assert!(rows_text(&unset).contains("Calendar (iCal link)             not set"));
    }

    #[test]
    fn rows_dim_while_meeting_mode_is_off_and_the_old_help_is_kept() {
        let (_dir, _path, mut state) = state_for("");
        let specs = state.form.row_specs();
        let enabled = specs.iter().find(|r| r.label == "Meeting mode").unwrap();
        assert!(!enabled.dimmed);
        assert!(specs
            .iter()
            .filter(|r| !r.heading && r.label != "Meeting mode")
            .all(|r| r.dimmed));
        state.form.focus_key("meeting.enabled");
        let page = screen(&state, 160, 40);
        assert!(page.contains("Long-form recording mode"), "{page}");
        state.form.handle_key(key(KeyCode::Right));
        assert!(state.form.row_specs().iter().all(|r| !r.dimmed));
    }

    #[test]
    #[ignore = "prints the rendered page for review"]
    fn dump_the_page() {
        let (_dir, _path, mut state) = state_for("[meeting]\nenabled = true\n");
        state.form.focus_key("meeting.audio.echo_cancel");
        println!("{}", screen(&state, 150, 50));
        println!("{}", rows_text(&state));
    }
}
