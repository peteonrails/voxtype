//! Dictation history in the GTK4 window helper.
//!
//! The window talks only to `voxtype history --json`. Its layout and
//! behaviour follow `quickshell/voxtype-meetings/HistoryPanel.qml`, built
//! from stock GTK widgets so the colours come from the GTK theme.
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use chrono::Datelike;
use gtk4::accessible::Property;
use gtk4::prelude::*;
use gtk4::{gdk, gio, glib, pango};
use serde_json::Value;

/// How often a visible window asks for new dictations.
const POLL: Duration = Duration::from_secs(2);
/// How long a notice such as "Copied." stays up.
const NOTICE: Duration = Duration::from_millis(3500);
/// The pause after the last keystroke before a search runs.
const SEARCH_DELAY_MS: u32 = 300;
/// Days of history offered in the header; 0 keeps everything.
const RETENTION_CHOICES: [u32; 5] = [7, 30, 90, 365, 0];
const SEPARATOR: &str = "  \u{b7}  ";
const EDIT_HINT: &str = "Enter saves  \u{b7}  Esc cancels  \u{b7}  Shift+Enter starts a new line. \
                         The original wording is kept.";

/// Sizes and surfaces only. Every colour is the theme's own: the text
/// colour at a strength, or one of the theme's named colours.
const CSS: &str = "
.history-row { border-radius: 6px; padding: 6px 6px 6px 10px; background-color: alpha(currentColor, 0.04); }
.history-row:hover, .history-row.active { background-color: alpha(currentColor, 0.08); }
.history-row.editing { box-shadow: inset 0 0 0 1px @theme_selected_bg_color; }
.history-row .history-actions { opacity: 0.55; }
.history-row:hover .history-actions, .history-row.active .history-actions { opacity: 1; }
.history-transcript { font-size: 115%; }
textview.history-transcript { border-radius: 4px; }
.history-chip { min-height: 0; padding: 2px 8px; font-size: 90%; font-weight: normal; }
.history-chip.history-danger:hover, .history-bad { color: mix(@error_color, @theme_fg_color, 0.35); }
.history-state { border: 1px solid alpha(currentColor, 0.45); border-radius: 9px; padding: 0 8px; font-size: 85%; font-weight: normal; }
.history-accent { color: mix(@theme_selected_bg_color, @theme_fg_color, 0.4); }
";

#[derive(Clone)]
struct HistoryClient {
    config_path: Option<PathBuf>,
    view: Rc<ViewRequests>,
}

#[derive(Default)]
struct ViewRequests {
    generation: Cell<u64>,
    query: RefCell<String>,
    closed: Cell<bool>,
}

impl ViewRequests {
    fn invalidate(&self, query: String) {
        self.generation.set(self.generation.get().wrapping_add(1));
        self.query.replace(query);
    }

    fn request(self: &Rc<Self>) -> ListRequest {
        let query = self.query.borrow().clone();
        self.invalidate(query.clone());
        ListRequest {
            view: self.clone(),
            generation: self.generation.get(),
            query,
        }
    }
}

struct ListRequest {
    view: Rc<ViewRequests>,
    generation: u64,
    query: String,
}

impl ListRequest {
    fn is_current(&self) -> bool {
        !self.view.closed.get() && self.generation == self.view.generation.get()
    }

    fn searching(&self) -> bool {
        !self.query.trim().is_empty()
    }
}

impl HistoryClient {
    fn command(config_path: Option<PathBuf>) -> std::process::Command {
        let sibling = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.join("voxtype")));
        let executable = sibling
            .filter(|p| p.is_file())
            .unwrap_or_else(|| PathBuf::from("voxtype"));
        let mut command = std::process::Command::new(executable);
        if let Some(path) = config_path {
            command.arg("--config").arg(path);
        }
        command.stdin(std::process::Stdio::null());
        command
    }

    /// Run a blocking command on its own thread, off the GTK main loop.
    async fn off_thread<T: Send + 'static>(
        name: &str,
        work: impl FnOnce() -> Result<T, String> + Send + 'static,
    ) -> Result<T, String> {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let _thread = std::thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                let _ = sender.send(work());
            })
            .map_err(|error| format!("Could not start history request: {error}"))?;
        receiver
            .await
            .map_err(|_| "History request worker stopped".to_string())?
    }

    async fn open_meeting(&self, id: String) -> Result<Value, String> {
        let config_path = self.config_path.clone();
        Self::off_thread("voxtype-history-meeting", move || {
            let result = Self::command(config_path)
                .args(["meeting", "view", &id])
                .output()
                .map_err(|error| error.to_string())?;
            if result.status.success() {
                Ok(Value::Null)
            } else {
                Err(format!(
                    "Meeting launch failed: {}",
                    String::from_utf8_lossy(&result.stderr)
                ))
            }
        })
        .await
    }

    async fn history(&self, args: &[&str]) -> Result<Value, String> {
        self.request(args.iter().map(|arg| (*arg).to_string()).collect())
            .await
    }

    async fn request(&self, args: Vec<String>) -> Result<Value, String> {
        let config_path = self.config_path.clone();
        Self::off_thread("voxtype-history-io", move || {
            Self::history_blocking(config_path, args)
        })
        .await
    }

    fn history_blocking(config_path: Option<PathBuf>, args: Vec<String>) -> Result<Value, String> {
        let result = Self::command(config_path)
            .args(["history", "--json"])
            .args(args)
            .output()
            .map_err(|e| e.to_string())?;
        let value: Value = serde_json::from_slice(&result.stdout).map_err(|_| {
            format!(
                "History command did not return JSON: {}",
                String::from_utf8_lossy(&result.stderr)
            )
        })?;
        if !result.status.success() {
            return Err(value["error"]
                .as_str()
                .unwrap_or("History request failed")
                .to_string());
        }
        Ok(value)
    }
}

fn spawn_ui(anchor: &gtk4::Widget, work: impl Future<Output = ()> + 'static) {
    let hold = anchor
        .root()
        .and_then(|root| root.downcast::<gtk4::Window>().ok())
        .and_then(|window| window.application())
        .map(|app| app.hold());
    let _task = glib::MainContext::default().spawn_local(async move {
        let _hold = hold;
        work.await;
    });
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or("")
}

/// The correction, else what was output, else what the engine heard.
fn best_text(entry: &Value) -> &str {
    if let Some(corrected) = entry["corrected_text"].as_str() {
        return corrected;
    }
    let output = text(entry, "text");
    if output.is_empty() {
        text(entry, "raw_text")
    } else {
        output
    }
}

/// A length of audio, such as "6.1 s" or "2 min 5 s". Empty for none.
fn seconds(ms: u64) -> String {
    if ms == 0 {
        String::new()
    } else if ms < 60_000 {
        format!("{:.1} s", ms as f64 / 1000.0)
    } else {
        let rest = ((ms % 60_000) as f64 / 1000.0).round();
        format!("{} min {rest} s", ms / 60_000)
    }
}

/// The parts that have something in them, with a dot between.
fn joined<S: AsRef<str>>(parts: &[S]) -> String {
    let kept: Vec<&str> = parts
        .iter()
        .map(AsRef::as_ref)
        .filter(|part| !part.is_empty())
        .collect();
    kept.join(SEPARATOR)
}

fn retention_label(days: u32) -> String {
    match days {
        0 => "forever".into(),
        1 => "1 day".into(),
        365 => "1 year".into(),
        days => format!("{days} days"),
    }
}

/// The drop-down's choices: the usual ones, plus the configured value when
/// the config file holds something else.
fn retention_choices(current: u32) -> Vec<u32> {
    let mut choices = RETENTION_CHOICES.to_vec();
    if !choices.contains(&current) {
        let at = choices
            .iter()
            .position(|choice| *choice == 0 || *choice > current)
            .unwrap_or(choices.len());
        choices.insert(at, current);
    }
    choices
}

/// "Today · Monday, November 16", "Yesterday · ...", then the plain date.
fn day_label(day: &str, today: chrono::NaiveDate) -> String {
    let Ok(date) = chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d") else {
        return day.to_string();
    };
    // GLib names the day and month in the user's language. Noon, because
    // midnight does not exist on some daylight-saving days.
    let name = glib::DateTime::from_local(
        date.year(),
        date.month() as i32,
        date.day() as i32,
        12,
        0,
        0.0,
    )
    .and_then(|local| local.format("%A, %B %-d"))
    .map(|name| name.to_string())
    .unwrap_or_else(|_| date.format("%A, %B %-d").to_string());
    match (today - date).num_days() {
        0 => format!("Today{SEPARATOR}{name}"),
        1 => format!("Yesterday{SEPARATOR}{name}"),
        _ => name,
    }
}

struct Detail {
    label: &'static str,
    value: String,
    mono: bool,
}

/// The expanded view of one dictation, as label and value pairs.
fn detail_rows(entry: &Value) -> Vec<Detail> {
    fn plain(label: &'static str, value: impl Into<String>) -> Detail {
        Detail {
            label,
            value: value.into(),
            mono: false,
        }
    }
    let mut out = Vec::new();
    let raw = text(entry, "raw_text");
    let output = text(entry, "text");
    let correction = entry["corrected_text"].as_str();
    out.push(plain(
        "Raw:",
        if raw.is_empty() {
            "(nothing heard)"
        } else {
            raw
        },
    ));
    if output != raw || correction.is_some() {
        out.push(plain("Output:", output));
    }
    if let Some(correction) = correction {
        out.push(plain(
            "Correction:",
            if correction.is_empty() {
                "(empty)"
            } else {
                correction
            },
        ));
    }
    if let Some(edits) = entry["edits"].as_array().filter(|edits| !edits.is_empty()) {
        let mut diff = Vec::new();
        for edit in edits {
            let (from, to) = (text(edit, "from"), text(edit, "to"));
            if !from.is_empty() {
                diff.push(format!("- {from}"));
            }
            if !to.is_empty() {
                diff.push(format!("+ {to}"));
            }
        }
        out.push(Detail {
            label: "Automatic edits:",
            value: diff.join("\n"),
            mono: true,
        });
    }
    out.push(plain(
        "Engine:",
        joined(&[
            text(entry, "engine"),
            text(entry, "model"),
            text(entry, "variant"),
            text(entry, "backend"),
            text(entry, "language"),
        ]),
    ));
    let labelled = |prefix: &str, value: &str| {
        if value.is_empty() {
            String::new()
        } else {
            format!("{prefix}{value}")
        }
    };
    let flag = |key: &str, name: &str| {
        if entry[key] == true {
            name.to_string()
        } else {
            String::new()
        }
    };
    let audio = seconds(entry["audio_ms"].as_u64().unwrap_or(0));
    let timing = joined(&[
        if audio.is_empty() {
            audio
        } else {
            format!("{audio} of audio")
        },
        entry["transcribe_ms"]
            .as_u64()
            .map(|ms| format!("{ms} ms to transcribe"))
            .unwrap_or_default(),
        entry["model_load_ms"]
            .as_u64()
            .filter(|ms| *ms > 0)
            .map(|ms| format!("{ms} ms to load the model"))
            .unwrap_or_default(),
        flag("streaming", "streaming"),
    ]);
    if !timing.is_empty() {
        out.push(plain("Timing:", timing));
    }
    let session = joined(&[
        labelled("started by ", text(entry, "trigger")),
        labelled("ended by ", text(entry, "ended_by")),
        labelled("profile ", text(entry, "profile")),
        labelled("mic ", text(entry, "input_device")),
        flag("auto_submit", "auto-submit"),
    ]);
    if !session.is_empty() {
        out.push(plain("Session:", session));
    }
    let result = joined(&[text(entry, "outcome"), text(entry, "error")]);
    if !result.is_empty() {
        out.push(plain("Result:", result));
    }
    out.push(Detail {
        label: "ID:",
        value: text(entry, "id").to_string(),
        mono: true,
    });
    out
}

/// One row of the list: a dictation, or a meeting that matched a search.
#[derive(Clone, Default, PartialEq)]
struct Item {
    /// What the row is known by. Unique in a list.
    id: String,
    /// The meeting to open, for a meeting row.
    meeting: Option<String>,
    day: String,
    time: String,
    meta: String,
    failed: bool,
    corrected: bool,
    transcript: String,
    /// The whole dictation, for the details. Null for a meeting.
    entry: Value,
}

impl Item {
    fn from_hit(hit: &Value) -> Result<Self, String> {
        if hit["source"] == "meeting" {
            let id = hit["meeting_id"]
                .as_str()
                .ok_or("Meeting search result has no ID")?;
            let date = text(hit, "date");
            let title = text(hit, "title");
            return Ok(Self {
                id: format!("meeting:{id}"),
                meeting: Some(id.to_string()),
                day: date.get(..10).unwrap_or(date).to_string(),
                time: "Meeting".into(),
                meta: if title.is_empty() { "Meeting" } else { title }.to_string(),
                transcript: text(hit, "snippet_plain").to_string(),
                ..Self::default()
            });
        }
        let entry = if hit["entry"].is_object() {
            &hit["entry"]
        } else {
            hit
        };
        let id = entry["id"].as_str().ok_or("History entry has no ID")?;
        let start = text(entry, "ts_start");
        let or = |key: &str, fallback: &'static str| {
            let value = text(entry, key);
            if value.is_empty() {
                fallback.to_string()
            } else {
                value.to_string()
            }
        };
        Ok(Self {
            id: id.to_string(),
            meeting: None,
            day: start.get(..10).unwrap_or(start).to_string(),
            time: start.get(11..16).unwrap_or("").to_string(),
            meta: joined(&[
                or("app", "unknown app"),
                or("model", "unknown model"),
                seconds(entry["audio_ms"].as_u64().unwrap_or(0)),
            ]),
            failed: entry["delivered"] != true,
            corrected: entry["corrected_text"].is_string(),
            transcript: best_text(entry).to_string(),
            entry: entry.clone(),
        })
    }

    /// The rows for a `list` or `search` reply.
    fn from_hits(hits: &[Value]) -> Result<Vec<Self>, String> {
        let mut seen = HashMap::<String, u32>::new();
        let mut items = Vec::with_capacity(hits.len());
        for hit in hits {
            let mut item = Self::from_hit(hit)?;
            // A search can return one meeting twice. Rows are told apart by
            // ID, so the repeats get their own.
            let count = seen.entry(item.id.clone()).or_default();
            *count += 1;
            if *count > 1 {
                item.id = format!("{}#{count}", item.id);
            }
            items.push(item);
        }
        Ok(items)
    }
}

/// The offer, after a correction that changed one phrase, to always make
/// that change.
#[derive(Clone)]
struct RuleOffer {
    id: String,
    from: String,
    to: String,
}

fn label(text: &str) -> gtk4::Label {
    let label = gtk4::Label::new(Some(text));
    label.set_xalign(0.0);
    label.set_wrap(true);
    label.set_wrap_mode(pango::WrapMode::WordChar);
    label
}

fn small(text: &str, classes: &[&str]) -> gtk4::Label {
    let label = label(text);
    label.add_css_class("caption");
    for class in classes {
        label.add_css_class(class);
    }
    label
}

/// A small flat button.
fn chip(text: &str) -> gtk4::Button {
    let button = gtk4::Button::with_label(text);
    button.add_css_class("flat");
    button.add_css_class("history-chip");
    button.set_valign(gtk4::Align::Start);
    button
}

/// Run `action` on the panel, for the row `id`, when `button` is clicked.
fn on_click(button: &gtk4::Button, panel: &Rc<Panel>, id: &str, action: fn(&Rc<Panel>, &str)) {
    let (panel, id) = (Rc::downgrade(panel), id.to_string());
    button.connect_clicked(move |_| {
        if let Some(panel) = panel.upgrade() {
            action(&panel, &id);
        }
    });
}

/// The character under a point in a label, counted in characters.
fn offset_at(label: &gtk4::Label, x: f64, y: f64) -> i32 {
    let (left, top) = label.layout_offsets();
    let (_, index, trailing) = label.layout().xy_to_index(
        (x as i32 - left) * pango::SCALE,
        (y as i32 - top) * pango::SCALE,
    );
    let text = label.text();
    let before = text
        .get(..index.max(0) as usize)
        .map_or(0, |before| before.chars().count());
    before as i32 + trailing
}

/// The widgets of one row. They stay for as long as the dictation is in
/// the list, so a refresh changes their text and nothing else.
struct Row {
    item: RefCell<Item>,
    root: gtk4::Box,
    day: gtk4::Label,
    card: gtk4::Box,
    time: gtk4::Label,
    text_line: gtk4::Box,
    text: gtk4::Label,
    editor: RefCell<Option<gtk4::TextView>>,
    actions: gtk4::Stack,
    view_actions: gtk4::Box,
    edit_actions: gtk4::Box,
    edit: gtk4::Button,
    copy: gtk4::Button,
    paste: gtk4::Button,
    details_button: gtk4::Button,
    hint: gtk4::Label,
    offer: gtk4::Box,
    offer_text: gtk4::Label,
    meta: gtk4::Label,
    failed: gtk4::Label,
    corrected: gtk4::Label,
    details: gtk4::Box,
}

impl Row {
    fn new(panel: &Rc<Panel>, id: &str, meeting: bool) -> Rc<Self> {
        let root = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .accessible_role(gtk4::AccessibleRole::ListItem)
            .build();
        let day = label("");
        day.add_css_class("caption-heading");
        day.add_css_class("history-accent");
        day.set_margin_top(10);
        day.set_margin_bottom(4);
        day.set_margin_start(4);
        day.set_visible(false);
        root.append(&day);

        let card = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        card.add_css_class("history-row");
        root.append(&card);

        let time = small("", &["dim-label"]);
        time.set_wrap(false);
        time.set_valign(gtk4::Align::Start);
        time.set_margin_top(6);
        time.set_width_request(60);
        card.append(&time);

        let column = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
        column.set_hexpand(true);
        card.append(&column);

        // The transcript, with the actions beside it.
        let text_line = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        column.append(&text_line);
        let text = label("");
        text.add_css_class("history-transcript");
        text.set_hexpand(true);
        text.set_valign(gtk4::Align::Start);
        // The same insets as the editor that replaces it, so the words do
        // not move when a correction starts.
        text.set_margin_top(3);
        text.set_margin_bottom(3);
        text.set_margin_start(4);
        text.set_margin_end(4);
        text_line.append(&text);

        // A stack, so the row's actions and the editor's Cancel and Save
        // take the same room: the words keep their width, and their line
        // breaks, when a correction starts.
        let actions = gtk4::Stack::new();
        actions.add_css_class("history-actions");
        actions.set_valign(gtk4::Align::Start);
        text_line.append(&actions);
        let view_actions = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        let edit_actions = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        edit_actions.set_halign(gtk4::Align::End);
        actions.add_child(&view_actions);
        actions.add_child(&edit_actions);

        let edit = chip("Edit");
        let copy = chip("Copy");
        let paste = chip("Paste again");
        let details_button = chip("Details");
        let delete = chip("Delete");
        delete.add_css_class("history-danger");
        if meeting {
            let open = chip("Open meeting");
            on_click(&open, panel, id, Panel::open_meeting);
            view_actions.append(&open);
        } else {
            on_click(&edit, panel, id, |panel, id| {
                panel.begin_correction(id, None)
            });
            on_click(&copy, panel, id, Panel::copy);
            on_click(&paste, panel, id, paste_dialog);
            on_click(&details_button, panel, id, Panel::toggle_details);
            on_click(&delete, panel, id, delete_dialog);
            for button in [&edit, &copy, &paste, &details_button, &delete] {
                view_actions.append(button);
            }
            let cancel = chip("Cancel");
            let save = chip("Save");
            on_click(&cancel, panel, id, |panel, _| panel.cancel_correction());
            on_click(&save, panel, id, |panel, _| panel.save_correction());
            edit_actions.append(&cancel);
            edit_actions.append(&save);

            // Click the words to correct them, with the caret where you
            // clicked.
            text.set_cursor_from_name(Some("text"));
            let click = gtk4::GestureClick::new();
            click.set_button(gdk::BUTTON_PRIMARY);
            let (weak, row_id) = (Rc::downgrade(panel), id.to_string());
            click.connect_released(move |gesture, _, x, y| {
                let Some(label) = gesture
                    .widget()
                    .and_then(|widget| widget.downcast::<gtk4::Label>().ok())
                else {
                    return;
                };
                let offset = offset_at(&label, x, y);
                let (weak, row_id) = (weak.clone(), row_id.clone());
                // Not from inside the gesture: starting hides its label.
                glib::idle_add_local_once(move || {
                    if let Some(panel) = weak.upgrade() {
                        panel.begin_correction(&row_id, Some(offset));
                    }
                });
            });
            text.add_controller(click);
        }

        let hint = small(EDIT_HINT, &["dim-label"]);
        hint.set_margin_start(4);
        hint.set_visible(false);
        column.append(&hint);

        let offer = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        offer.set_margin_start(4);
        offer.set_visible(false);
        let offer_text = small("", &["history-accent"]);
        offer_text.set_hexpand(true);
        offer.append(&offer_text);
        let add = chip("Add replacement");
        let dismiss = chip("Not now");
        on_click(&add, panel, id, |panel, _| panel.add_rule());
        on_click(&dismiss, panel, id, |panel, _| panel.set_offer(None));
        offer.append(&add);
        offer.append(&dismiss);
        column.append(&offer);

        // One small line: app, model and audio length, then the marks.
        let meta_line = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        meta_line.set_margin_start(4);
        let meta = small("", &["dim-label"]);
        meta.set_wrap(false);
        meta.set_ellipsize(pango::EllipsizeMode::End);
        meta.set_hexpand(true);
        let failed = small("[failed]", &["history-bad"]);
        let corrected = small("[corrected]", &["history-accent"]);
        failed.set_visible(false);
        corrected.set_visible(false);
        meta_line.append(&meta);
        meta_line.append(&failed);
        meta_line.append(&corrected);
        column.append(&meta_line);

        let details = gtk4::Box::new(gtk4::Orientation::Vertical, 3);
        details.set_margin_top(6);
        details.set_margin_bottom(4);
        details.set_margin_start(4);
        details.set_visible(false);
        column.append(&details);

        Rc::new(Self {
            item: RefCell::default(),
            root,
            day,
            card,
            time,
            text_line,
            text,
            editor: RefCell::default(),
            actions,
            view_actions,
            edit_actions,
            edit,
            copy,
            paste,
            details_button,
            hint,
            offer,
            offer_text,
            meta,
            failed,
            corrected,
            details,
        })
    }

    fn id(&self) -> String {
        self.item.borrow().id.clone()
    }

    /// Show `item`. Returns whether anything changed.
    fn apply(&self, item: Item) -> bool {
        if *self.item.borrow() == item {
            return false;
        }
        self.time.set_text(&item.time);
        if item.transcript.is_empty() && item.failed {
            self.text.set_text("No text");
            self.text.add_css_class("dim-label");
        } else {
            self.text.set_text(&item.transcript);
            self.text.remove_css_class("dim-label");
        }
        self.meta.set_text(&item.meta);
        self.failed.set_visible(item.failed);
        self.corrected.set_visible(item.corrected);
        self.copy.set_sensitive(!item.transcript.is_empty());
        self.paste.set_sensitive(!item.transcript.is_empty());
        let details_changed = self.item.borrow().entry != item.entry;
        self.item.replace(item);
        if details_changed && self.details.is_visible() {
            self.fill_details();
        }
        true
    }

    /// The day header above the row, for the first row of a day.
    fn set_day(&self, day: Option<String>) {
        self.day.set_visible(day.is_some());
        if let Some(day) = day.filter(|day| self.day.text() != *day) {
            self.day.set_text(&day);
        }
    }

    fn fill_details(&self) {
        while let Some(child) = self.details.first_child() {
            self.details.remove(&child);
        }
        for detail in detail_rows(&self.item.borrow().entry) {
            let line = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
            let key = small(detail.label, &["dim-label"]);
            key.set_wrap(false);
            key.set_valign(gtk4::Align::Start);
            key.set_width_request(112);
            let value = small(&detail.value, &[]);
            value.set_hexpand(true);
            value.set_selectable(true);
            if detail.mono {
                value.add_css_class("monospace");
            }
            line.append(&key);
            line.append(&value);
            self.details.append(&line);
        }
    }

    fn set_expanded(&self, expanded: bool) {
        if self.details.is_visible() == expanded {
            return;
        }
        self.details.set_visible(expanded);
        self.details_button
            .set_label(if expanded { "Hide details" } else { "Details" });
        self.mark_active();
        if expanded {
            self.fill_details();
        }
    }

    /// The row stays lit while it is open or being corrected.
    fn mark_active(&self) {
        if self.details.is_visible() || self.editor.borrow().is_some() {
            self.card.add_css_class("active");
        } else {
            self.card.remove_css_class("active");
        }
    }

    fn show_offer(&self, offer: Option<&RuleOffer>) {
        self.offer.set_visible(offer.is_some());
        if let Some(offer) = offer {
            self.offer_text.set_text(&format!(
                "Always type \u{201c}{}\u{201d} when Voxtype hears \u{201c}{}\u{201d}?",
                offer.to, offer.from
            ));
        }
    }

    /// The line under the editor: how to save, or why saving failed.
    fn set_hint(&self, error: &str) {
        if error.is_empty() {
            self.hint.set_text(EDIT_HINT);
            self.hint.remove_css_class("history-bad");
            self.hint.add_css_class("dim-label");
        } else {
            self.hint.set_text(error);
            self.hint.remove_css_class("dim-label");
            self.hint.add_css_class("history-bad");
        }
    }

    /// Swap the transcript for an editor, in place. `cursor` is where the
    /// caret goes, in characters; `None` puts it at the end.
    fn start_edit(&self, panel: &Rc<Panel>, cursor: Option<i32>) {
        let editor = gtk4::TextView::new();
        editor.add_css_class("history-transcript");
        editor.set_wrap_mode(gtk4::WrapMode::WordChar);
        editor.set_accepts_tab(false);
        editor.set_hexpand(true);
        editor.set_valign(gtk4::Align::Start);
        editor.set_top_margin(3);
        editor.set_bottom_margin(3);
        editor.set_left_margin(4);
        editor.set_right_margin(4);
        editor.update_property(&[Property::Label("Correction")]);
        // Start at the size of the words it replaces. A text view that is
        // given no height never measures its text, and stays that way.
        let insets = self.text.margin_top() + self.text.margin_bottom();
        editor.set_height_request(self.text.height() + insets);
        let buffer = editor.buffer();
        buffer.set_text(&self.item.borrow().transcript);

        // Enter saves and Esc cancels; Shift+Enter is left to the editor,
        // which starts a new line.
        let keys = gtk4::EventControllerKey::new();
        keys.set_propagation_phase(gtk4::PropagationPhase::Capture);
        let weak = Rc::downgrade(panel);
        keys.connect_key_pressed(move |controller, key, _, modifiers| {
            let save = matches!(
                key,
                gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::ISO_Enter
            ) && !modifiers.contains(gdk::ModifierType::SHIFT_MASK);
            if !save && key != gdk::Key::Escape {
                return glib::Propagation::Proceed;
            }
            // An input method that is composing text takes the key first.
            let composing = controller
                .widget()
                .and_then(|widget| widget.downcast::<gtk4::TextView>().ok())
                .zip(controller.current_event())
                .is_some_and(|(editor, event)| editor.im_context_filter_keypress(&event));
            if !composing {
                let weak = weak.clone();
                // Not from inside the key handler: both remove the editor.
                glib::idle_add_local_once(move || {
                    let Some(panel) = weak.upgrade() else { return };
                    if save {
                        panel.save_correction();
                    } else {
                        panel.cancel_correction();
                    }
                });
            }
            glib::Propagation::Stop
        });
        editor.add_controller(keys);

        self.text.set_visible(false);
        self.text_line.prepend(&editor);
        self.actions.set_visible_child(&self.edit_actions);
        self.set_hint("");
        self.hint.set_visible(true);
        self.card.add_css_class("editing");
        editor.grab_focus();
        let caret = match cursor {
            Some(offset) => buffer.iter_at_offset(offset),
            None => buffer.end_iter(),
        };
        buffer.place_cursor(&caret);
        self.editor.replace(Some(editor));
        self.mark_active();
    }

    fn stop_edit(&self) {
        let Some(editor) = self.editor.take() else {
            return;
        };
        let focused = editor.has_focus();
        self.text_line.remove(&editor);
        self.text.set_visible(true);
        self.actions.set_visible_child(&self.view_actions);
        self.hint.set_visible(false);
        self.card.remove_css_class("editing");
        self.mark_active();
        if focused {
            self.edit.grab_focus();
        }
    }

    fn edited_text(&self) -> Option<String> {
        let editor = self.editor.borrow();
        let buffer = editor.as_ref()?.buffer();
        Some(
            buffer
                .text(&buffer.start_iter(), &buffer.end_iter(), true)
                .to_string(),
        )
    }
}

/// What a frame clock signal runs.
type FrameHandler = Box<dyn Fn(&gdk::FrameClock)>;

/// One history window.
struct Panel {
    client: HistoryClient,
    window: gtk4::ApplicationWindow,
    state: gtk4::Label,
    retention: gtk4::DropDown,
    pause: gtk4::Button,
    enabled: gtk4::Button,
    error: gtk4::Label,
    notice: gtk4::Label,
    empty: gtk4::Label,
    scroll: gtk4::ScrolledWindow,
    list: gtk4::Box,
    rows: RefCell<Vec<Rc<Row>>>,
    /// The query the rows on screen answer; `None` before the first list.
    rendered_query: RefCell<Option<String>>,
    /// The query last asked for. It trails the search box while the user
    /// is still typing.
    requested_query: RefCell<Option<String>>,
    /// "History on", "History paused" or "History off", once known.
    state_text: RefCell<String>,
    history_enabled: Cell<bool>,
    history_paused: Cell<bool>,
    retention_days: Cell<u32>,
    /// The days behind each drop-down row.
    retention_rows: RefCell<Vec<u32>>,
    /// Set while the drop-down is being changed from here, not by the user.
    retention_quiet: Cell<bool>,
    expanded: RefCell<Option<String>>,
    correcting: RefCell<Option<String>>,
    offer: RefCell<Option<RuleOffer>>,
    /// An action is running. One at a time.
    pending: Cell<bool>,
    /// The generation of the list request that is running.
    list_pending: Cell<Option<u64>>,
    /// Open confirmations.
    dialogs: Cell<u32>,
    action_error: RefCell<String>,
    list_error: RefCell<String>,
    notice_serial: Cell<u32>,
}

impl Panel {
    fn new(app: &gtk4::Application, config_path: Option<PathBuf>) -> Rc<Self> {
        // Each configuration keeps its own immutable command context. A later
        // forwarded launch must never retarget another window's pending action.
        let client = HistoryClient {
            config_path,
            view: Rc::default(),
        };
        let window = gtk4::ApplicationWindow::builder()
            .application(app)
            .title("Voxtype History")
            .default_width(1000)
            .default_height(760)
            .build();
        window.set_widget_name("history");

        let header = gtk4::HeaderBar::new();
        let title = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        let name = gtk4::Label::new(Some("Voxtype History"));
        name.add_css_class("title");
        let state = gtk4::Label::new(None);
        state.add_css_class("history-state");
        state.set_valign(gtk4::Align::Center);
        state.set_visible(false);
        title.append(&name);
        title.append(&state);
        header.set_title_widget(Some(&title));

        let retention = gtk4::DropDown::from_strings(&[]);
        retention.set_tooltip_text(Some("How long dictations are kept"));
        retention.update_property(&[Property::Label("Keep dictations for")]);
        retention.set_visible(false);
        let pause = gtk4::Button::with_label("Pause");
        pause.set_visible(false);
        let enabled = gtk4::Button::with_label("Turn on");
        enabled.set_sensitive(false);
        header.pack_end(&enabled);
        header.pack_end(&pause);
        header.pack_end(&retention);
        window.set_titlebar(Some(&header));

        let content = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
        content.set_margin_top(12);
        content.set_margin_bottom(12);
        content.set_margin_start(12);
        content.set_margin_end(12);
        let search = gtk4::SearchEntry::new();
        search.set_placeholder_text(Some("Search dictations and meetings"));
        // The property arrived in GTK 4.8; older ones keep their own delay.
        if search.find_property("search-delay").is_some() {
            search.set_property("search-delay", SEARCH_DELAY_MS);
        }
        content.append(&search);
        let error = label("");
        error.add_css_class("history-bad");
        error.set_visible(false);
        content.append(&error);
        let notice = label("");
        notice.add_css_class("history-accent");
        notice.set_visible(false);
        content.append(&notice);
        let empty = label("");
        empty.add_css_class("dim-label");
        empty.set_visible(false);
        content.append(&empty);
        let list = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(3)
            .accessible_role(gtk4::AccessibleRole::List)
            .build();
        let scroll = gtk4::ScrolledWindow::builder()
            .vexpand(true)
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .child(&list)
            .build();
        content.append(&scroll);
        window.set_child(Some(&content));
        gtk4::prelude::GtkWindowExt::set_focus(&window, Some(&search));

        let panel = Rc::new(Self {
            client,
            window,
            state,
            retention,
            pause,
            enabled,
            error,
            notice,
            empty,
            scroll,
            list,
            rows: RefCell::default(),
            rendered_query: RefCell::default(),
            requested_query: RefCell::default(),
            state_text: RefCell::new("Loading history".into()),
            history_enabled: Cell::default(),
            history_paused: Cell::default(),
            retention_days: Cell::default(),
            retention_rows: RefCell::default(),
            retention_quiet: Cell::default(),
            expanded: RefCell::default(),
            correcting: RefCell::default(),
            offer: RefCell::default(),
            pending: Cell::default(),
            list_pending: Cell::default(),
            dialogs: Cell::default(),
            action_error: RefCell::default(),
            list_error: RefCell::default(),
            notice_serial: Cell::default(),
        });

        {
            let view = panel.client.view.clone();
            // Invalidate immediately, not only after SearchEntry's debounce.
            search.connect_changed(move |search| view.invalidate(search.text().to_string()));
        }
        let weak = Rc::downgrade(&panel);
        search.connect_search_changed({
            let weak = weak.clone();
            move |_| {
                // GTK also sends this once on its own, with nothing typed.
                if let Some(panel) = weak.upgrade().filter(|panel| panel.typing()) {
                    panel.refresh();
                }
            }
        });
        search.connect_activate({
            let weak = weak.clone();
            move |_| {
                if let Some(panel) = weak.upgrade() {
                    panel.refresh();
                }
            }
        });
        // Esc clears the search. Emptying the entry searches at once.
        search.connect_stop_search(|search| search.set_text(""));
        panel.pause.connect_clicked({
            let weak = weak.clone();
            move |_| {
                if let Some(panel) = weak.upgrade() {
                    let action = if panel.history_paused.get() {
                        "resume"
                    } else {
                        "pause"
                    };
                    panel.change_state(action);
                }
            }
        });
        panel.enabled.connect_clicked({
            let weak = weak.clone();
            move |_| {
                if let Some(panel) = weak.upgrade() {
                    let action = if panel.history_enabled.get() {
                        "disable"
                    } else {
                        "enable"
                    };
                    panel.change_state(action);
                }
            }
        });
        panel.retention.connect_selected_notify({
            let weak = weak.clone();
            move |retention| {
                let Some(panel) = weak.upgrade() else { return };
                if panel.retention_quiet.get() {
                    return;
                }
                let chosen = panel
                    .retention_rows
                    .borrow()
                    .get(retention.selected() as usize)
                    .copied();
                if let Some(days) = chosen.filter(|days| *days != panel.retention_days.get()) {
                    panel.set_retention(days);
                }
            }
        });
        // New dictations appear on their own while the window is showing.
        glib::timeout_add_local(POLL, move || {
            let Some(panel) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if panel.client.view.closed.get() {
                return glib::ControlFlow::Break;
            }
            if panel.window.is_visible() && panel.idle() {
                panel.refresh();
            }
            glib::ControlFlow::Continue
        });
        panel.refresh();
        panel
    }

    fn row(&self, id: &str) -> Option<Rc<Row>> {
        self.rows
            .borrow()
            .iter()
            .find(|row| row.item.borrow().id == id)
            .cloned()
    }

    fn closed(&self) -> bool {
        self.client.view.closed.get()
    }

    /// The search box holds something that has not been asked for yet.
    fn typing(&self) -> bool {
        self.requested_query.borrow().as_deref() != Some(self.client.view.query.borrow().as_str())
    }

    /// Nothing the user is in the middle of may be disturbed by a background
    /// refresh: a search or correction being typed, a confirmation, a
    /// running action.
    fn idle(&self) -> bool {
        // A list request that a newer one has replaced does not count: it
        // can be stuck behind a locked index for a long time.
        let listing = self.list_pending.get() == Some(self.client.view.generation.get());
        !listing
            && !self.typing()
            && !self.pending.get()
            && self.correcting.borrow().is_none()
            && self.dialogs.get() == 0
    }

    fn refresh(self: &Rc<Self>) {
        // Reserve before spawning, so a newer search cannot be superseded by
        // a late reply to this one.
        let request = self.client.view.request();
        self.list_pending.set(Some(request.generation));
        self.requested_query.replace(Some(request.query.clone()));
        let panel = self.clone();
        spawn_ui(self.window.upcast_ref(), async move {
            let result = if request.searching() {
                panel.client.history(&["search", &request.query]).await
            } else {
                panel.client.history(&["list"]).await
            };
            if panel.list_pending.get() == Some(request.generation) {
                panel.list_pending.set(None);
            }
            // Check before propagating failures as well as before touching
            // rows. A discarded response must not update the notice either.
            if !request.is_current() {
                return;
            }
            let shown = result.and_then(|value| panel.show_list(&request, &value));
            panel.list_error.replace(match shown {
                Ok(()) => String::new(),
                Err(error) if request.searching() => {
                    format!("History search did not complete: {error}")
                }
                Err(error) => format!("Could not read history: {error}"),
            });
            if panel.state_text.borrow().as_str() == "Loading history" {
                panel.show_state_text("Unavailable", "History state unavailable");
            }
            panel.show_messages();
        });
    }

    fn show_list(self: &Rc<Self>, request: &ListRequest, value: &Value) -> Result<(), String> {
        let hits = if request.searching() {
            value.as_array()
        } else {
            value["entries"].as_array()
        }
        .ok_or("History returned an invalid result list")?;
        let items = Item::from_hits(hits)?;
        if !request.searching() {
            self.apply_state(value);
        }
        self.sync_rows(items, &request.query);
        Ok(())
    }

    /// Bring the rows to `items` without rebuilding the ones that are still
    /// there, so the view keeps its place, open rows stay open and a
    /// correction being typed is left alone.
    fn sync_rows(self: &Rc<Self>, items: Vec<Item>, query: &str) {
        let new_query = self.rendered_query.borrow().as_deref() != Some(query);
        let anchor = if new_query { None } else { self.anchor() };
        let mut changed = false;
        {
            let mut rows = self.rows.borrow_mut();
            let kept: HashSet<&str> = items.iter().map(|item| item.id.as_str()).collect();
            rows.retain(|row| {
                let stays = kept.contains(row.item.borrow().id.as_str());
                if !stays {
                    self.list.remove(&row.root);
                    changed = true;
                }
                stays
            });
            for (n, item) in items.into_iter().enumerate() {
                let before = n.checked_sub(1).map(|n| rows[n].root.clone());
                let found = rows[n..]
                    .iter()
                    .position(|row| row.item.borrow().id == item.id);
                let row = match found {
                    Some(0) => rows[n].clone(),
                    Some(ahead) => {
                        let row = rows.remove(n + ahead);
                        self.list.reorder_child_after(&row.root, before.as_ref());
                        rows.insert(n, row.clone());
                        changed = true;
                        row
                    }
                    None => {
                        let row = Row::new(self, &item.id, item.meeting.is_some());
                        self.list.insert_child_after(&row.root, before.as_ref());
                        rows.insert(n, row.clone());
                        changed = true;
                        row
                    }
                };
                changed |= row.apply(item);
            }
        }

        let today = chrono::Local::now().date_naive();
        let expanded = self.expanded.borrow().clone();
        let offer = self.offer.borrow().clone();
        let mut previous = String::new();
        for row in self.rows.borrow().iter() {
            let item = row.item.borrow();
            row.set_day((item.day != previous).then(|| day_label(&item.day, today)));
            previous.clone_from(&item.day);
            row.set_expanded(expanded.as_deref() == Some(item.id.as_str()));
            row.show_offer(offer.as_ref().filter(|offer| offer.id == item.id));
        }
        let gone = |id: &RefCell<Option<String>>| {
            let id = id.borrow().clone();
            id.is_some_and(|id| self.row(&id).is_none())
        };
        if gone(&self.correcting) {
            self.correcting.replace(None);
            self.action_error.replace(String::new());
        }
        self.empty.set_text(if query.trim().is_empty() {
            "No dictations stored"
        } else {
            "No matches"
        });
        self.empty.set_visible(self.rows.borrow().is_empty());

        if new_query {
            // A different list: start at its top.
            self.scroll.vadjustment().set_value(0.0);
        } else if let Some((row, offset)) = anchor.filter(|_| changed) {
            self.keep_in_view(&row, offset);
        }
        self.rendered_query.replace(Some(query.to_string()));
    }

    /// The first row in view and how far down the view it starts, when the
    /// list is scrolled. At the top there is nothing to hold: new rows are
    /// meant to push the others down.
    fn anchor(&self) -> Option<(Rc<Row>, f64)> {
        let top = self.scroll.vadjustment().value();
        if top <= 0.0 {
            return None;
        }
        self.rows.borrow().iter().find_map(|row| {
            let bounds = row.root.compute_bounds(&self.list)?;
            let (y, height) = (f64::from(bounds.y()), f64::from(bounds.height()));
            (y + height > top).then(|| (row.clone(), y - top))
        })
    }

    /// After rows above it came or went, scroll so `row` is where it was.
    /// Rows only have their new places once GTK has laid the list out, so
    /// this waits for the next layout.
    fn keep_in_view(&self, row: &Rc<Row>, offset: f64) {
        let Some(clock) = self.list.frame_clock() else {
            return;
        };
        let (list, target, adjustment) = (
            self.list.clone(),
            row.root.clone(),
            self.scroll.vadjustment(),
        );
        // If the user scrolls first, their scroll wins.
        let before = adjustment.value();
        let restore = move || {
            if adjustment.value() != before || target.parent().is_none() {
                return;
            }
            if let Some(bounds) = target.compute_bounds(&list) {
                let value = f64::from(bounds.y()) - offset;
                if (value - before).abs() >= 1.0 {
                    adjustment.set_value(value);
                }
            }
        };
        // Layout is where the rows move; after-paint catches a frame whose
        // layout ran before this handler was reached.
        let once = |connect: &dyn Fn(FrameHandler) -> glib::SignalHandlerId| {
            let id = Rc::new(RefCell::new(None));
            let (slot, restore) = (id.clone(), restore.clone());
            id.replace(Some(connect(Box::new(move |clock| {
                restore();
                if let Some(id) = slot.take() {
                    clock.disconnect(id);
                }
            }))));
        };
        once(&|run| clock.connect_layout(run));
        once(&|run| clock.connect_after_paint(run));
    }

    fn show_state_text(&self, short: &str, full: &str) {
        self.state.set_text(short);
        self.state.set_visible(true);
        // The chip says "On"; a screen reader hears what is on.
        self.state.update_property(&[Property::Label(full)]);
        self.state_text.replace(full.to_string());
    }

    fn apply_state(&self, value: &Value) {
        let enabled = value["enabled"].as_bool().unwrap_or(false);
        let paused = value["paused"].as_bool().unwrap_or(false);
        self.history_enabled.set(enabled);
        self.history_paused.set(paused);
        if !enabled {
            self.show_state_text("Off", "History off");
            self.state.add_css_class("dim-label");
        } else {
            self.state.remove_css_class("dim-label");
            if paused {
                self.show_state_text("Paused", "History paused");
            } else {
                self.show_state_text("On", "History on");
            }
        }
        let (label, name) = if paused {
            ("Resume", "Resume history")
        } else {
            ("Pause", "Pause history")
        };
        self.pause.set_label(label);
        self.pause.update_property(&[Property::Label(name)]);
        self.pause.set_visible(enabled);
        let (label, name) = if enabled {
            ("Turn off", "Turn off history")
        } else {
            ("Turn on", "Turn on history")
        };
        self.enabled.set_label(label);
        self.enabled.update_property(&[Property::Label(name)]);
        self.enabled.set_sensitive(!self.pending.get());
        if let Some(days) = value["retention_days"]
            .as_u64()
            .and_then(|days| u32::try_from(days).ok())
        {
            self.show_retention(days);
        }
    }

    fn show_retention(&self, days: u32) {
        let choices = retention_choices(days);
        self.retention_quiet.set(true);
        if *self.retention_rows.borrow() != choices {
            let labels: Vec<String> = choices
                .iter()
                .map(|days| format!("Keep {}", retention_label(*days)))
                .collect();
            let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
            self.retention
                .set_model(Some(&gtk4::StringList::new(&labels)));
            self.retention_rows.replace(choices.clone());
        }
        let selected = choices.iter().position(|choice| *choice == days);
        if let Some(selected) = selected.filter(|at| self.retention.selected() as usize != *at) {
            self.retention.set_selected(selected as u32);
        }
        self.retention_quiet.set(false);
        self.retention_days.set(days);
        self.retention.set_visible(true);
    }

    /// Start one action, unless another is running. `done` gets the reply
    /// unless the window closed first. Returns whether the action started.
    fn act(
        self: &Rc<Self>,
        work: impl Future<Output = Result<Value, String>> + 'static,
        done: impl FnOnce(&Rc<Self>, Result<Value, String>) + 'static,
    ) -> bool {
        if self.pending.replace(true) {
            return false;
        }
        self.set_error("");
        self.set_busy(true);
        let panel = self.clone();
        spawn_ui(self.window.upcast_ref(), async move {
            let result = work.await;
            panel.pending.set(false);
            if panel.closed() {
                return;
            }
            panel.set_busy(false);
            done(&panel, result);
        });
        true
    }

    fn history(
        self: &Rc<Self>,
        args: &[&str],
        done: impl FnOnce(&Rc<Self>, Result<Value, String>) + 'static,
    ) -> bool {
        let client = self.client.clone();
        let args = args.iter().map(|arg| (*arg).to_string()).collect();
        self.act(async move { client.request(args).await }, done)
    }

    fn set_busy(&self, busy: bool) {
        for control in [
            self.pause.upcast_ref::<gtk4::Widget>(),
            self.enabled.upcast_ref(),
            self.retention.upcast_ref(),
        ] {
            control.set_sensitive(!busy);
        }
    }

    fn set_error(&self, text: &str) {
        self.action_error.replace(text.to_string());
        self.show_messages();
    }

    /// An error from a correction shows under its editor; any other shows
    /// above the list, where a list that could not be read also reports.
    fn show_messages(&self) {
        let error = self.action_error.borrow();
        let editing = self.correcting.borrow().clone();
        let editing = editing.and_then(|id| self.row(&id));
        if let Some(row) = &editing {
            row.set_hint(&error);
        }
        let list_error = self.list_error.borrow();
        let shown = if editing.is_none() && !error.is_empty() {
            error.as_str()
        } else {
            list_error.as_str()
        };
        self.error.set_text(shown);
        self.error.set_visible(!shown.is_empty());
    }

    fn set_notice(self: &Rc<Self>, text: &str) {
        let serial = self.notice_serial.get().wrapping_add(1);
        self.notice_serial.set(serial);
        self.notice.set_text(text);
        self.notice.set_visible(true);
        let panel = Rc::downgrade(self);
        glib::timeout_add_local_once(NOTICE, move || {
            if let Some(panel) = panel.upgrade().filter(|p| p.notice_serial.get() == serial) {
                panel.notice.set_visible(false);
            }
        });
    }

    fn change_state(self: &Rc<Self>, action: &str) {
        self.history(&[action], |panel, result| match result {
            Ok(value) if value["enabled"].is_boolean() => {
                panel.apply_state(&value);
                panel.refresh();
            }
            Ok(_) => panel.set_error("History change did not complete"),
            Err(error) => panel.set_error(&format!("History change did not complete: {error}")),
        });
    }

    fn set_retention(self: &Rc<Self>, days: u32) {
        let started = self.history(&["retention", &days.to_string()], |panel, result| {
            match result {
                Ok(value) if value["enabled"].is_boolean() => {
                    panel.apply_state(&value);
                    panel.refresh();
                }
                Ok(_) => panel.set_error("Retention change did not complete"),
                Err(error) => {
                    panel.set_error(&format!("Retention change did not complete: {error}"));
                }
            }
            // Whatever happened, the drop-down shows what is in force.
            panel.show_retention(panel.retention_days.get());
        });
        if !started {
            self.show_retention(self.retention_days.get());
        }
    }

    fn begin_correction(self: &Rc<Self>, id: &str, cursor: Option<i32>) {
        if self.pending.get() || self.correcting.borrow().as_deref() == Some(id) {
            return;
        }
        let Some(row) = self.row(id) else { return };
        self.end_correction();
        self.set_offer(None);
        // A click on "No text" is not a place in the transcript.
        let cursor = cursor.filter(|_| !row.item.borrow().transcript.is_empty());
        self.correcting.replace(Some(id.to_string()));
        row.start_edit(self, cursor);
    }

    fn end_correction(&self) {
        let id = self.correcting.take();
        if let Some(row) = id.and_then(|id| self.row(&id)) {
            row.stop_edit();
        }
        self.set_error("");
    }

    fn cancel_correction(&self) {
        if !self.pending.get() {
            self.end_correction();
        }
    }

    fn save_correction(self: &Rc<Self>) {
        let id = self.correcting.borrow().clone();
        let Some(row) = id.and_then(|id| self.row(&id)) else {
            return;
        };
        let Some(text) = row.edited_text() else {
            return;
        };
        if self.pending.get() {
            return;
        }
        // Nothing changed: leave the dictation unmarked.
        if text == row.item.borrow().transcript {
            self.end_correction();
            return;
        }
        let id = row.id();
        self.history(
            &["correct", &id.clone(), "--", &text],
            move |panel, result| match result {
                Err(error) => panel.set_error(&format!("Correction did not complete: {error}")),
                Ok(value) => {
                    panel.end_correction();
                    let rule = &value["rule_suggestion"];
                    panel.set_offer(rule["from"].as_str().zip(rule["to"].as_str()).map(
                        |(from, to)| RuleOffer {
                            id,
                            from: from.to_string(),
                            to: to.to_string(),
                        },
                    ));
                    panel.refresh();
                }
            },
        );
    }

    fn set_offer(&self, offer: Option<RuleOffer>) {
        for row in self.rows.borrow().iter() {
            let shown = offer
                .as_ref()
                .filter(|offer| offer.id == row.item.borrow().id);
            row.show_offer(shown);
        }
        self.offer.replace(offer);
    }

    fn add_rule(self: &Rc<Self>) {
        let Some(offer) = self.offer.borrow().clone() else {
            return;
        };
        self.history(
            &["rule", "--", &offer.from, &offer.to],
            |panel, result| match result {
                Err(error) => panel.set_error(&format!("The replacement was not added: {error}")),
                Ok(value) => {
                    panel.set_offer(None);
                    panel.set_notice(if value["restart_required"] == true {
                        "Replacement added. It applies the next time Voxtype starts."
                    } else {
                        "Replacement added."
                    });
                }
            },
        );
    }

    fn copy(self: &Rc<Self>, id: &str) {
        self.history(&["copy", id], |panel, result| match result {
            Ok(_) => panel.set_notice("Copied."),
            Err(error) => panel.set_error(&format!("Copy did not complete: {error}")),
        });
    }

    fn open_meeting(self: &Rc<Self>, id: &str) {
        let Some(meeting) = self
            .row(id)
            .and_then(|row| row.item.borrow().meeting.clone())
        else {
            return;
        };
        let client = self.client.clone();
        self.act(
            async move { client.open_meeting(meeting).await },
            |panel, result| match result {
                Ok(_) => panel.set_notice("Opening the meeting."),
                Err(error) => panel.set_error(&error),
            },
        );
    }

    fn toggle_details(self: &Rc<Self>, id: &str) {
        let open = self.expanded.borrow().as_deref() != Some(id);
        self.expanded.replace(open.then(|| id.to_string()));
        for row in self.rows.borrow().iter() {
            row.set_expanded(open && row.item.borrow().id == id);
        }
    }

    /// A confirmation window over this one. Background refreshes wait until
    /// it closes.
    fn dialog(self: &Rc<Self>, title: &str, message: &str) -> (gtk4::Window, gtk4::Box) {
        let window = gtk4::Window::builder()
            .title(title)
            .transient_for(&self.window)
            .modal(true)
            .destroy_with_parent(true)
            .default_width(540)
            .build();
        window.set_application(self.window.application().as_ref());
        let body = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
        body.set_margin_top(18);
        body.set_margin_bottom(18);
        body.set_margin_start(18);
        body.set_margin_end(18);
        body.append(&label(message));
        window.set_child(Some(&body));
        self.dialogs.set(self.dialogs.get() + 1);
        let panel = Rc::downgrade(self);
        window.connect_close_request(move |_| {
            if let Some(panel) = panel.upgrade() {
                panel.dialogs.set(panel.dialogs.get().saturating_sub(1));
            }
            glib::Propagation::Proceed
        });
        (window, body)
    }
}

fn delete_dialog(panel: &Rc<Panel>, id: &str) {
    let (window, body) = panel.dialog(
        "Delete dictation",
        "Delete this dictation from its day file and search index?",
    );
    let notice = label("");
    notice.add_css_class("error");
    notice.set_visible(false);
    body.append(&notice);
    let actions = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    let cancel = gtk4::Button::with_label("Cancel deletion");
    let confirm = gtk4::Button::with_label("Delete dictation");
    confirm.add_css_class("destructive-action");
    actions.append(&cancel);
    actions.append(&confirm);
    body.append(&actions);
    {
        let window = window.clone();
        cancel.connect_clicked(move |_| window.close());
    }
    {
        let (window, panel, id) = (window.clone(), panel.clone(), id.to_string());
        confirm.connect_clicked(move |button| {
            button.set_sensitive(false);
            let (window, panel, id, notice, button) = (
                window.clone(),
                panel.clone(),
                id.clone(),
                notice.clone(),
                button.clone(),
            );
            let anchor = window.clone();
            spawn_ui(anchor.upcast_ref(), async move {
                let result = panel.client.history(&["delete", &id]).await;
                if panel.closed() {
                    return;
                }
                match result {
                    Ok(_) => {
                        window.close();
                        // Refresh the live query, not the one the row was
                        // made under.
                        panel.refresh();
                    }
                    Err(error) => {
                        button.set_sensitive(true);
                        notice.set_text(&format!("Deletion action did not complete: {error}"));
                        notice.set_visible(true);
                    }
                }
            });
        });
    }
    window.present();
}

fn paste_dialog(panel: &Rc<Panel>, id: &str) {
    let (window, body) = panel.dialog(
        "Paste dictation",
        "History will hide before delivery. Focus the target window during the delay. \
         No new history entry is recorded.",
    );
    let actions = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    let cancel = gtk4::Button::with_label("Cancel replay");
    let confirm = gtk4::Button::with_label("Paste dictation");
    actions.append(&cancel);
    actions.append(&confirm);
    body.append(&actions);
    {
        let window = window.clone();
        cancel.connect_clicked(move |_| window.close());
    }
    {
        let (window, panel, id) = (window.clone(), panel.clone(), id.to_string());
        confirm.connect_clicked(move |_| {
            let parent = panel.window.clone();
            // Acquire before unmapping: an unmapped widget need not retain
            // its root, and the idle delay must not outlive the application.
            let hold = parent.application().map(|app| app.hold());
            window.close();
            parent.set_visible(false);
            let (panel, id) = (panel.clone(), id.clone());
            // Let GTK unmap the history window before starting the CLI. Keep
            // the application alive so closing the view cannot kill replay.
            glib::timeout_add_local_once(Duration::from_millis(250), move || {
                let anchor = parent.clone();
                spawn_ui(anchor.upcast_ref(), async move {
                    let _hold = hold;
                    let result = panel
                        .client
                        .history(&["paste", &id, "--delay-ms", "500"])
                        .await;
                    if panel.closed() {
                        return;
                    }
                    if let Err(error) = result {
                        panel.set_error(&format!("Replay did not complete: {error}"));
                        parent.present();
                    }
                });
            });
        });
    }
    window.present();
}

fn install_css() {
    let Some(display) = gdk::Display::default() else {
        return;
    };
    let provider = gtk4::CssProvider::new();
    provider.load_from_data(CSS);
    gtk4::style_context_add_provider_for_display(
        &display,
        &provider,
        gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}

pub(super) fn run() -> glib::ExitCode {
    let mut flags = gio::ApplicationFlags::HANDLES_COMMAND_LINE;
    if std::env::var_os(super::SMOKE_ENV).is_some() {
        flags |= gio::ApplicationFlags::NON_UNIQUE;
    }
    let app = gtk4::Application::builder()
        .application_id("io.voxtype.History")
        .flags(flags)
        .build();
    app.connect_startup(|_| install_css());
    let existing = Rc::new(RefCell::new(HashMap::<Option<PathBuf>, Rc<Panel>>::new()));
    app.connect_command_line(move |app, command_line| {
        let arguments = command_line.arguments();
        let config_path = arguments
            .windows(2)
            .find(|pair| pair[0] == "--config")
            .map(|pair| {
                let path = PathBuf::from(&pair[1]);
                if path.is_absolute() {
                    Ok(path)
                } else {
                    // GApplication forwards the caller's cwd with its arguments.
                    // Direct helper invocations need the same isolation as the
                    // canonical CLI, without changing the primary's global cwd.
                    command_line.cwd().map(|cwd| cwd.join(path)).ok_or(
                        "Cannot resolve relative --config: invocation working directory is unavailable",
                    )
                }
            })
            .transpose();
        let config_path = match config_path {
            Ok(path) => path,
            Err(error) => {
                eprintln!("{error}");
                return glib::ExitCode::FAILURE;
            }
        };
        if let Some(panel) = existing.borrow().get(&config_path) {
            panel.window.present();
            if panel.idle() {
                panel.refresh();
            }
            return glib::ExitCode::SUCCESS;
        }
        let panel = Panel::new(app, config_path.clone());
        {
            let (existing, key, view) = (
                existing.clone(),
                config_path.clone(),
                panel.client.view.clone(),
            );
            panel.window.connect_close_request(move |_| {
                view.closed.set(true);
                view.invalidate(String::new());
                existing.borrow_mut().remove(&key);
                glib::Propagation::Proceed
            });
        }
        if let Some(seconds) = std::env::var(super::SMOKE_ENV)
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
        {
            let (panel, app) = (panel.clone(), app.clone());
            glib::timeout_add_local_once(Duration::from_secs(seconds), move || {
                let ids: Vec<String> = panel.rows.borrow().iter().map(|row| row.id()).collect();
                println!(
                    "{}",
                    serde_json::json!({
                        "screen": panel.window.widget_name().to_string(),
                        "state": panel.state_text.borrow().as_str(),
                        "retention_days": panel.retention_days.get(),
                        "entries": ids.len(),
                        "ids": ids,
                    })
                );
                if let Some(path) = std::env::var_os(super::SHOT_ENV) {
                    super::save_png(&panel.window, std::path::Path::new(&path));
                }
                app.quit();
            });
        }
        panel.window.present();
        existing.borrow_mut().insert(config_path, panel);
        glib::ExitCode::SUCCESS
    });
    app.run()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn date(text: &str) -> chrono::NaiveDate {
        chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn audio_length_reads_in_seconds_then_minutes() {
        assert_eq!(seconds(0), "");
        assert_eq!(seconds(6080), "6.1 s");
        assert_eq!(seconds(59_900), "59.9 s");
        assert_eq!(seconds(125_400), "2 min 5 s");
    }

    #[test]
    fn joined_skips_empty_parts() {
        assert_eq!(joined(&["Slack", "", "6.1 s"]), "Slack  \u{b7}  6.1 s");
        assert_eq!(joined(&["", ""]), "");
    }

    #[test]
    fn retention_reads_as_a_length_of_time() {
        assert_eq!(retention_label(0), "forever");
        assert_eq!(retention_label(1), "1 day");
        assert_eq!(retention_label(30), "30 days");
        assert_eq!(retention_label(365), "1 year");
    }

    #[test]
    fn retention_choices_include_a_configured_value_in_order() {
        assert_eq!(retention_choices(30), [7, 30, 90, 365, 0]);
        assert_eq!(retention_choices(0), [7, 30, 90, 365, 0]);
        assert_eq!(retention_choices(14), [7, 14, 30, 90, 365, 0]);
        assert_eq!(retention_choices(3), [3, 7, 30, 90, 365, 0]);
        assert_eq!(retention_choices(730), [7, 30, 90, 365, 730, 0]);
    }

    #[test]
    fn day_headers_name_today_and_yesterday() {
        let today = date("2026-11-16");
        assert_eq!(
            day_label("2026-11-16", today),
            "Today  \u{b7}  Monday, November 16"
        );
        assert_eq!(
            day_label("2026-11-15", today),
            "Yesterday  \u{b7}  Sunday, November 15"
        );
        assert_eq!(day_label("2026-11-09", today), "Monday, November 9");
        assert_eq!(day_label("not a day", today), "not a day");
    }

    #[test]
    fn a_dictation_becomes_a_two_line_row() {
        let item = Item::from_hit(&json!({
            "id": "20261116-155531-efe557",
            "ts_start": "2026-11-16T15:55:31.318946-05:00",
            "raw_text": "Add me to the cone ogres file.",
            "text": "Add me to the cone ogres file.",
            "corrected_text": "Add me to the CODEOWNERS file.",
            "model": "parakeet-ultra-Q8_0.gguf",
            "audio_ms": 2660,
            "delivered": true,
            "app": "com.mitchellh.ghostty",
        }))
        .unwrap();
        assert_eq!(item.day, "2026-11-16");
        assert_eq!(item.time, "15:55");
        assert_eq!(item.transcript, "Add me to the CODEOWNERS file.");
        assert_eq!(
            item.meta,
            "com.mitchellh.ghostty  \u{b7}  parakeet-ultra-Q8_0.gguf  \u{b7}  2.7 s"
        );
        assert!(item.corrected && !item.failed);
    }

    #[test]
    fn a_failed_dictation_without_an_app_is_marked() {
        let item = Item::from_hit(&json!({
            "id": "a", "ts_start": "2026-11-16T09:00:00+00:00", "raw_text": "um", "delivered": false,
        }))
        .unwrap();
        assert!(item.failed && !item.corrected);
        assert_eq!(item.transcript, "um");
        assert_eq!(item.meta, "unknown app  \u{b7}  unknown model");
    }

    #[test]
    fn search_hits_unwrap_and_repeated_meetings_stay_apart() {
        let items = Item::from_hits(&[
            json!({"source": "dictation", "entry": {"id": "a", "ts_start": "2026-11-16T09:00:00+00:00", "text": "one", "delivered": true}}),
            json!({"source": "meeting", "meeting_id": "m1", "title": "Planning", "date": "2026-11-15T10:00:00Z", "snippet_plain": "the budget"}),
            json!({"source": "meeting", "meeting_id": "m1", "title": "Planning", "date": "2026-11-15T10:00:00Z", "snippet_plain": "more budget"}),
        ])
        .unwrap();
        let ids: Vec<&str> = items.iter().map(|item| item.id.as_str()).collect();
        assert_eq!(ids, ["a", "meeting:m1", "meeting:m1#2"]);
        assert_eq!(items[2].meeting.as_deref(), Some("m1"));
        assert_eq!(items[1].time, "Meeting");
        assert_eq!(items[1].meta, "Planning");
        assert_eq!(items[1].day, "2026-11-15");
        assert!(Item::from_hits(&[json!({"source": "meeting"})]).is_err());
    }

    #[test]
    fn details_are_a_short_labelled_list() {
        let rows = detail_rows(&json!({
            "id": "20261116-171431-8412c3",
            "raw_text": "um can you send me the q3 numbers",
            "text": "Can you send me the Q3 numbers",
            "edits": [{"from": "um", "to": ""}, {"from": "q3", "to": "Q3"}],
            "engine": "parakeet", "model": "parakeet-ultra-Q8_0.gguf", "variant": "vulkan", "language": "en",
            "audio_ms": 6080, "transcribe_ms": 92, "model_load_ms": 0, "streaming": false,
            "trigger": "record", "ended_by": "released", "input_device": "default", "auto_submit": false,
            "outcome": "delivered",
        }));
        let shown: Vec<(&str, &str)> = rows
            .iter()
            .map(|row| (row.label, row.value.as_str()))
            .collect();
        assert_eq!(
            shown,
            [
                ("Raw:", "um can you send me the q3 numbers"),
                ("Output:", "Can you send me the Q3 numbers"),
                ("Automatic edits:", "- um\n- q3\n+ Q3"),
                (
                    "Engine:",
                    "parakeet  \u{b7}  parakeet-ultra-Q8_0.gguf  \u{b7}  vulkan  \u{b7}  en"
                ),
                ("Timing:", "6.1 s of audio  \u{b7}  92 ms to transcribe"),
                (
                    "Session:",
                    "started by record  \u{b7}  ended by released  \u{b7}  mic default"
                ),
                ("Result:", "delivered"),
                ("ID:", "20261116-171431-8412c3"),
            ]
        );
        assert!(rows[2].mono && rows[7].mono && !rows[0].mono);
    }

    #[test]
    fn details_leave_out_what_matches_and_show_a_correction() {
        let same = detail_rows(&json!({"id": "a", "raw_text": "hello", "text": "hello"}));
        let labels: Vec<&str> = same.iter().map(|row| row.label).collect();
        assert_eq!(labels, ["Raw:", "Engine:", "ID:"]);

        let corrected = detail_rows(&json!({
            "id": "a", "raw_text": "", "text": "", "corrected_text": "", "outcome": "no_speech", "error": "nothing heard",
        }));
        let shown: Vec<(&str, &str)> = corrected
            .iter()
            .map(|row| (row.label, row.value.as_str()))
            .collect();
        assert_eq!(
            shown,
            [
                ("Raw:", "(nothing heard)"),
                ("Output:", ""),
                ("Correction:", "(empty)"),
                ("Engine:", ""),
                ("Result:", "no_speech  \u{b7}  nothing heard"),
                ("ID:", "a"),
            ]
        );
    }
}
