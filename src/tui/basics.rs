//! Basics: the handful of settings most people change, on the first page.
//!
//! Every row is a setting that lives on another page (Engine, Hotkey,
//! Output, On-screen display). Basics doesn't keep its own copy: it shows
//! that page's row and passes keys to that page with its cursor on the
//! same setting, so a change made here is the change on that page.
//!
//! Below them, problems that stop dictation from working appear as rows
//! too, each fixed with Enter.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::Rect,
    text::{Line, Span},
    Frame,
};

use super::app::{Action, App};
use super::common::{self, FormRowSpec};
use super::section::Section;
use super::theme;
use super::{engine, hotkey, osd_section, output_section};
use crate::setup::variant_check::Remediation;

/// One row on Basics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Row {
    /// A setting owned by another page, found there by its label.
    Setting(Section, &'static str),
    /// The installed build can't run the configured engine.
    FixBuild,
    /// The configured model isn't downloaded.
    Download,
}

const SETTINGS: &[(Section, &str)] = &[
    (Section::Engine, "Engine"),
    (Section::Engine, "Model"),
    (Section::Hotkey, "Hotkey"),
    (Section::Hotkey, "Hotkey mode"),
    (Section::Output, "Output"),
    (Section::Osd, "On-screen display"),
];

fn row_list(app: &App) -> Vec<Row> {
    let mut list: Vec<Row> = SETTINGS
        .iter()
        .map(|(section, label)| Row::Setting(*section, label))
        .collect();
    if app.variant_mismatch.is_some() {
        list.push(Row::FixBuild);
    }
    if app.missing_model.is_some() {
        list.push(Row::Download);
    }
    list
}

fn current(app: &App) -> Row {
    let list = row_list(app);
    list[app.basics_cursor.min(list.len() - 1)]
}

/// The page that owns the setting under the cursor, so keys and the
/// "typing a value" state go to it.
pub fn target_section(app: &App) -> Section {
    match current(app) {
        Row::Setting(section, _) => section,
        Row::FixBuild | Row::Download => Section::Basics,
    }
}

/// True when the cursor is on a problem row, where Enter fixes it.
pub fn on_problem_row(app: &App) -> bool {
    match current(app) {
        Row::Download => true,
        // A source build is fixed by rebuilding, which Enter can't do.
        Row::FixBuild => app
            .variant_mismatch
            .as_ref()
            .is_some_and(|m| matches!(m.remediation, Remediation::SwitchToVariant { .. })),
        Row::Setting(..) => false,
    }
}

/// The owning page's row for `label`, if that page loaded.
fn setting_row(app: &App, section: Section, label: &str) -> Option<FormRowSpec> {
    app.section_rows(section)?
        .into_iter()
        .find(|r| r.label == label)
}

pub fn rows(app: &App) -> Vec<FormRowSpec> {
    let cursor = app.basics_cursor;
    row_list(app)
        .into_iter()
        .enumerate()
        .map(|(i, row)| {
            let focused = i == cursor;
            match row {
                Row::Setting(section, label) => {
                    let mut spec = setting_row(app, section, label).unwrap_or_else(|| {
                        FormRowSpec::new(false, label, "Can't read").dimmed(true)
                    });
                    spec.focused = focused;
                    // With the listener off the key does nothing; say what
                    // starts dictation instead.
                    if label == "Hotkey" && app.hotkey.as_ref().is_some_and(|h| !h.enabled) {
                        spec.value = "Your desktop's shortcut".to_string();
                    }
                    spec
                }
                Row::FixBuild => {
                    let engine = app
                        .variant_mismatch
                        .as_ref()
                        .map(|m| engine::display_engine(m.configured_engine))
                        .unwrap_or("this engine");
                    FormRowSpec::new(focused, "Build", format!("Can't run {}", engine)).attention()
                }
                Row::Download => {
                    FormRowSpec::new(focused, "Model file", "Not downloaded").attention()
                }
            }
        })
        .collect()
}

/// Unsaved markers: a setting row is changed when its own page says so.
pub fn modified(app: &App, rows: &[FormRowSpec]) -> Vec<bool> {
    row_list(app)
        .into_iter()
        .take(rows.len())
        .map(|row| match row {
            Row::Setting(section, label) => setting_row(app, section, label)
                .is_some_and(|own| app.row_modified(section, label, &own.value)),
            _ => false,
        })
        .collect()
}

fn description(app: &App) -> Vec<Line<'static>> {
    let text = |s: &str| Line::from(s.to_string());
    match current(app) {
        Row::Setting(Section::Engine, "Engine") => {
            let mut lines = vec![text("The engine turns your speech into text.")];
            if let Some(state) = &app.engine {
                lines.push(text(engine::engine_summary(&state.engine)));
                if let Some((blocked, note)) = engine::engine_switch_note(state) {
                    lines.push(Line::from(Span::styled(
                        note,
                        if blocked {
                            theme::error()
                        } else {
                            theme::warn()
                        },
                    )));
                }
            }
            lines
        }
        Row::Setting(Section::Engine, _) => {
            let mut lines = vec![text("Larger models are more accurate and slower.")];
            if let Some(state) = &app.engine {
                if let Some(model) = setting_row(app, Section::Engine, "Model") {
                    lines.push(text(&engine::model_note(&state.engine, &model.value)));
                }
            }
            lines
        }
        Row::Setting(Section::Hotkey, "Hotkey") => {
            if app.hotkey.as_ref().is_some_and(|h| !h.enabled) {
                vec![text(
                    "Your desktop's own shortcut runs voxtype, because the built-in \
                     listener is off. Turn it on in Hotkey to use a key here.",
                )]
            } else {
                vec![text(
                    "The key that starts dictation. ←→ goes through common choices; \
                     Enter types any key name, such as F9 or RIGHTALT.",
                )]
            }
        }
        Row::Setting(Section::Hotkey, _) => vec![text(
            "Hold to talk records while the key is down. Toggle starts on one \
             press and stops on the next.",
        )],
        Row::Setting(Section::Output, _) => vec![text(
            "Type sends keystrokes to the focused window. Clipboard only copies \
             the text. Paste copies it and presses the paste keys. File writes \
             it to a file, set on the Output page.",
        )],
        Row::Setting(_, _) => vec![text(
            "A small overlay that shows voxtype is listening. Its look and \
             position are on the On-screen display page.",
        )],
        Row::FixBuild => fix_build_lines(app),
        Row::Download => {
            let model = app
                .missing_model
                .as_ref()
                .map(|m| m.model.clone())
                .unwrap_or_default();
            vec![text(&format!(
                "voxtype can't transcribe until {} is downloaded. Enter downloads \
                 it now, then restarts voxtype.",
                model
            ))]
        }
    }
}

fn fix_build_lines(app: &App) -> Vec<Line<'static>> {
    let Some(m) = &app.variant_mismatch else {
        return Vec::new();
    };
    let engine = engine::display_engine(m.configured_engine);
    match &m.remediation {
        Remediation::SwitchToVariant { target } => vec![Line::from(format!(
            "The installed voxtype build doesn't include {}. Enter switches to {}; \
             it asks for your password, then restarts voxtype.",
            engine,
            super::build::variant_name(*target)
        ))],
        Remediation::Rebuild { feature } => vec![Line::from(format!(
            "This voxtype was built without {}. Rebuild it with: \
             cargo build --release --features {}",
            engine, feature
        ))],
    }
}

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    common::render_form_with_guidance(f, area, app, None, &rows(app), description(app));
}

/// Put the owning page's cursor on the setting under Basics' cursor, so
/// the keys passed to it act on that setting.
fn focus_owner(app: &mut App) {
    let Row::Setting(section, label) = current(app) else {
        return;
    };
    match section {
        Section::Engine => {
            if let Some(s) = app.engine.as_mut() {
                engine::focus_label(s, label);
            }
        }
        Section::Hotkey => {
            if let Some(s) = app.hotkey.as_mut() {
                hotkey::focus_label(s, label);
            }
        }
        Section::Output => {
            if let Some(s) = app.output.as_mut() {
                output_section::focus_label(s, label);
            }
        }
        Section::Osd => {
            if let Some(s) = app.osd.as_mut() {
                osd_section::focus_label(s, label);
            }
        }
        _ => {}
    }
}

pub fn handle_key(app: &mut App, key: KeyEvent) -> Action {
    if !app.is_editing() {
        let len = row_list(app).len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                app.basics_cursor = app.basics_cursor.saturating_sub(1);
                return Action::None;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                app.basics_cursor = (app.basics_cursor + 1).min(len - 1);
                return Action::None;
            }
            // Reverting Basics reverts the pages it shows.
            KeyCode::Char('r') => {
                for_basics_pages(app);
                return Action::None;
            }
            _ => {}
        }
    }
    match current(app) {
        Row::Setting(section, _) => {
            focus_owner(app);
            match section {
                Section::Engine => engine::handle_key(app, key),
                Section::Hotkey => hotkey::handle_key(app, key),
                Section::Output => output_section::handle_key(app, key),
                Section::Osd => osd_section::handle_key(app, key),
                _ => Action::None,
            }
        }
        Row::FixBuild if key.code == KeyCode::Enter => fix_build(app),
        Row::Download if key.code == KeyCode::Enter => match &app.missing_model {
            Some(m) => Action::DownloadModel {
                engine: m.engine.clone(),
                model: m.model.clone(),
            },
            None => Action::None,
        },
        _ => Action::None,
    }
}

fn for_basics_pages(app: &mut App) {
    if let Some(s) = app.engine.as_mut().filter(|s| s.dirty_since_load) {
        s.reset();
    }
    if let Some(s) = app.hotkey.as_mut().filter(|s| s.dirty_since_load) {
        s.reset();
    }
    if let Some(s) = app.output.as_mut().filter(|s| s.dirty_since_load) {
        s.reset();
    }
    if let Some(s) = app.osd.as_mut().filter(|s| s.dirty_since_load) {
        s.reset();
    }
}

/// Enter on the build problem: switch to the build that can run the engine
/// when that is possible here, otherwise open the Build page.
fn fix_build(app: &mut App) -> Action {
    let target = match app.variant_mismatch.as_ref().map(|m| &m.remediation) {
        Some(Remediation::SwitchToVariant { target }) => *target,
        _ => return Action::None,
    };
    let list = super::build::listed(&app.inventory);
    if let Some(i) = list.iter().position(|v| *v == target) {
        app.build_cursor = i;
        if super::build::selected_switchable(app) == Some(target) {
            return Action::SwitchVariant(target);
        }
    }
    app.jump_to_section(Section::Build);
    Action::None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basics_lists_the_six_settings_first() {
        let labels: Vec<&str> = SETTINGS.iter().map(|(_, l)| *l).collect();
        assert_eq!(
            labels,
            [
                "Engine",
                "Model",
                "Hotkey",
                "Hotkey mode",
                "Output",
                "On-screen display"
            ]
        );
    }
}
