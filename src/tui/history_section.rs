//! History collection and privacy settings, using the canonical file-layer form.
use super::app::{Action, App};
use super::common::FeedbackLevel;
use super::config_editor::{ConfigEditor, EditorError};
use super::schema_form::{FormOptions, SchemaForm};
use crate::config::schema;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::Rect,
    widgets::{Block, Borders, Paragraph},
    Frame,
};

pub struct HistoryState {
    pub form: SchemaForm,
}

impl HistoryState {
    pub fn load() -> Result<Self, EditorError> {
        Ok(Self::from_editor(&ConfigEditor::load()?))
    }

    pub fn from_editor(editor: &ConfigEditor) -> Self {
        let keys = schema::scalar_keys()
            .filter(|key| key.key.starts_with("history."))
            .collect();
        Self {
            form: SchemaForm::new(
                FormOptions {
                    title: "History",
                    group_title: |_| "Collection, retention and privacy".into(),
                    master: None,
                    extra_help: |key| {
                        match key {
                "history.enabled" => vec!["Original engine text, processed text and outcomes are stored privately. Existing configs remain off until you enable history."],
                "history.exclude_apps" => vec!["Exclusions require a known app. Unsupported desktops, failed lookups and browser-hosted password managers need an explicit pause before sensitive dictation."],
                "history.retention_days" => vec!["0 keeps entries forever. Removing entries does not erase filesystem snapshots or backups."],
                _ => Vec::new(),
            }
                    },
                },
                keys,
                Vec::new(),
                editor,
            ),
        }
    }

    pub fn save(&mut self) -> Action {
        match ConfigEditor::load() {
            Ok(mut editor) => self.form.save_with_editor(&mut editor),
            Err(error) => self.form.feedback = Some((FeedbackLevel::Err, format!("load: {error}"))),
        }
        Action::None
    }

    pub fn reset(&mut self) {
        match ConfigEditor::load() {
            Ok(editor) => self.form.revert(&editor),
            Err(error) => self.form.feedback = Some((FeedbackLevel::Err, format!("load: {error}"))),
        }
    }
}

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    if let Some(state) = &app.history {
        state.form.render(frame, area);
    } else {
        frame.render_widget(
            Paragraph::new(
                "Could not load history settings. Check the config file and reopen this section.",
            )
            .block(Block::default().borders(Borders::ALL).title("History")),
            area,
        );
    }
}

pub fn handle_key(app: &mut App, key: KeyEvent) -> Action {
    let Some(state) = app.history.as_mut() else {
        return Action::None;
    };
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
    use crossterm::event::KeyModifiers;

    #[test]
    fn history_settings_can_persist_an_explicit_empty_exclusion_list() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("config.toml");
        std::fs::write(
            &path,
            "# private fixture\n[history]\nexclude_apps = [\"sensitive-app\"]\n",
        )
        .unwrap();
        let mut editor = ConfigEditor::load_from_path(path.clone()).unwrap();
        let mut state = HistoryState::from_editor(&editor);
        assert!(state.form.focus_key("history.exclude_apps"));
        state
            .form
            .handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        state
            .form
            .handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        state
            .form
            .handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        state.form.save_with_editor(&mut editor);
        let saved = std::fs::read_to_string(path).unwrap();
        let config: crate::config::Config = toml::from_str(&saved).unwrap();
        assert!(
            config.history.exclude_apps.is_empty(),
            "Empty input must not restore the password-manager defaults"
        );
        assert!(
            !config.history.enabled,
            "Editing privacy settings must not opt an old config in"
        );
        assert!(saved.contains("# private fixture"));
        assert_eq!(
            schema::resolve("history.exclude_apps", &config),
            Some(serde_json::json!(""))
        );
    }
}
