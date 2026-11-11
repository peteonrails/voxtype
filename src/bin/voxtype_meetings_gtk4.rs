//! `voxtype-meetings-gtk4`: the meeting view as a normal GTK4 window.
//!
//! For desktops without Quickshell (GNOME, Plasma, Sway, X11). It draws the
//! shared view model in `voxtype::meeting::view`, fed by the daemon's
//! `meeting.sock`, and every button runs the matching `voxtype meeting`
//! command, so the window holds no meeting logic of its own.
//!
//! An ordinary xdg-toplevel, not layer-shell: a meeting view is something
//! you move, resize and alt-tab to, and GNOME has no layer-shell. Plain
//! GTK4, no libadwaita.
//!
//! One instance per session: a second launch (from `voxtype meeting view`)
//! hands its arguments to the running window through GApplication.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use gtk4::prelude::*;
use gtk4::{gio, glib};

use voxtype::config::Config;
use voxtype::meeting::storage::MeetingListEntry;
use voxtype::meeting::view::client::{spawn_feed, Feed, FeedHandle};
use voxtype::meeting::view::commands::{self, AskAnswer, Cli, Outcome, Request};
use voxtype::meeting::view::{
    clock, escape_markup, line_markup, Change, MeetingView, ViewState, SPEAKER_COLORS,
};
use voxtype::meeting::{MeetingConfig, StorageConfig};

const APP_ID: &str = "io.voxtype.Meetings";

/// Set to a number of seconds to print the window's state as JSON and quit:
/// the headless smoke test drives the real window this way.
const SMOKE_ENV: &str = "VOXTYPE_MEETINGS_GTK4_SMOKE";

/// What a launch asked for.
#[derive(Debug, Default, Clone)]
struct Launch {
    meeting: Option<String>,
    tab: Option<String>,
    ask_id: Option<String>,
    ask: Option<String>,
}

fn parse_launch(args: &[String]) -> Launch {
    let mut l = Launch::default();
    let mut it = args.iter().skip(1);
    while let Some(a) = it.next() {
        let mut value = || it.next().cloned();
        match a.as_str() {
            "--meeting" => l.meeting = value(),
            "--tab" => l.tab = value(),
            "--ask-id" => l.ask_id = value(),
            "--ask" => l.ask = value(),
            "--config" => {
                if let Some(p) = value() {
                    Config::set_path_override(PathBuf::from(p));
                }
            }
            "--version" => {
                println!("voxtype-meetings-gtk4 {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            _ => {}
        }
    }
    l
}

/// Messages from worker threads.
enum Msg {
    Done(Outcome),
    /// A speaker chip was clicked.
    Rename(String),
    AskLine(String),
    AskExited(bool),
}

/// Widgets the refresh code updates.
struct Ui {
    window: gtk4::ApplicationWindow,
    stack: gtk4::Stack,
    state_label: gtk4::Label,
    title_label: gtk4::Label,
    elapsed_label: gtk4::Label,
    back_button: gtk4::Button,
    pause_button: gtk4::Button,
    stop_button: gtk4::Button,
    transcript: gtk4::ListBox,
    transcript_scroll: gtk4::ScrolledWindow,
    follow_button: gtk4::Button,
    chips: gtk4::Box,
    notes: gtk4::TextView,
    summary: gtk4::Label,
    ask_entry: gtk4::Entry,
    ask_answer: gtk4::Label,
    list: gtk4::ListBox,
    status: gtk4::Label,
}

/// Everything the window knows.
struct State {
    meetings: MeetingConfig,
    cli: Cli,
    live: MeetingView,
    stored: Option<MeetingView>,
    list: Vec<MeetingListEntry>,
    follow: bool,
    /// Segment id of the selected line.
    selected: Option<u32>,
    rendered_revision: Option<(bool, u64, Option<String>)>,
    ask: Option<AskAnswer>,
    notes_dirty: bool,
    notes_loaded_for: Option<String>,
    notes_seen_at: Option<chrono::DateTime<chrono::Utc>>,
    tx: Sender<Msg>,
    feed: Option<FeedHandle>,
}

impl State {
    fn view(&self) -> &MeetingView {
        self.stored.as_ref().unwrap_or(&self.live)
    }

    fn meeting_id(&self) -> Option<String> {
        self.view().meeting_id.clone()
    }

    fn run(&self, request: Request) {
        let cli = self.cli.clone();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Msg::Done(cli.run(&request)));
        });
    }
}

type Shared = Rc<RefCell<State>>;

/// The window once it exists, for later launches to reuse.
type Running = Rc<RefCell<Option<(Rc<Ui>, Shared)>>>;

fn meeting_config() -> MeetingConfig {
    let config =
        voxtype::config::load_config(Config::path_override().as_deref()).unwrap_or_default();
    let storage_path = if config.meeting.storage_path == "auto" {
        StorageConfig::default_storage_path()
    } else {
        PathBuf::from(&config.meeting.storage_path)
    };
    MeetingConfig {
        storage: StorageConfig {
            storage_path,
            ..StorageConfig::default()
        },
        ..MeetingConfig::default()
    }
}

fn load_stored(meetings: &MeetingConfig, id: &str) -> Option<MeetingView> {
    voxtype::meeting::get_meeting(meetings, id)
        .ok()
        .map(|m| MeetingView::stored(&m))
}

fn label(text: &str) -> gtk4::Label {
    let l = gtk4::Label::new(Some(text));
    l.set_xalign(0.0);
    l
}

fn heading(text: &str) -> gtk4::Label {
    let l = label("");
    l.set_markup(&format!("<b>{}</b>", escape_markup(text)));
    l.set_margin_top(8);
    l
}

fn build_ui(app: &gtk4::Application) -> Ui {
    let window = gtk4::ApplicationWindow::builder()
        .application(app)
        .title("Meetings - Voxtype")
        .default_width(980)
        .default_height(680)
        .build();

    // Header bar: back, state, title, elapsed; Pause and Stop.
    let header = gtk4::HeaderBar::new();
    let back_button = gtk4::Button::with_label("Meetings");
    let pause_button = gtk4::Button::with_label("Pause");
    let stop_button = gtk4::Button::with_label("Stop");
    stop_button.add_css_class("destructive-action");
    header.pack_start(&back_button);
    header.pack_end(&stop_button);
    header.pack_end(&pause_button);
    let title_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
    let state_label = gtk4::Label::new(None);
    let title_label = gtk4::Label::new(None);
    let elapsed_label = gtk4::Label::new(None);
    title_box.append(&state_label);
    title_box.append(&title_label);
    title_box.append(&elapsed_label);
    header.set_title_widget(Some(&title_box));
    window.set_titlebar(Some(&header));

    // The meeting: transcript on the left, speakers/notes/summary/ask right.
    let transcript = gtk4::ListBox::new();
    transcript.set_selection_mode(gtk4::SelectionMode::Single);
    let transcript_scroll = gtk4::ScrolledWindow::builder()
        .child(&transcript)
        .vexpand(true)
        .hexpand(true)
        .build();
    let line_bar = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    let move_button = gtk4::Button::with_label("Move line to…");
    let note_button = gtk4::Button::with_label("Note on line…");
    let follow_button = gtk4::Button::with_label("Follow new lines");
    line_bar.append(&move_button);
    line_bar.append(&note_button);
    line_bar.append(&follow_button);
    let left = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
    left.append(&transcript_scroll);
    left.append(&line_bar);

    let right = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    right.set_margin_start(8);
    right.set_margin_end(8);
    right.set_width_request(300);
    right.append(&heading("Speakers"));
    let chips = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
    right.append(&chips);
    right.append(&heading("Notes"));
    let notes = gtk4::TextView::new();
    notes.set_wrap_mode(gtk4::WrapMode::WordChar);
    let notes_scroll = gtk4::ScrolledWindow::builder()
        .child(&notes)
        .min_content_height(100)
        .build();
    right.append(&notes_scroll);
    let save_notes = gtk4::Button::with_label("Save notes");
    right.append(&save_notes);
    right.append(&heading("Summary"));
    let summary = label("");
    summary.set_wrap(true);
    summary.set_selectable(true);
    right.append(&summary);
    right.append(&heading("Ask"));
    let ask_entry = gtk4::Entry::new();
    ask_entry.set_placeholder_text(Some("Ask about this meeting"));
    right.append(&ask_entry);
    let ask_answer = label("");
    ask_answer.set_wrap(true);
    ask_answer.set_selectable(true);
    let ask_scroll = gtk4::ScrolledWindow::builder()
        .child(&ask_answer)
        .vexpand(true)
        .build();
    right.append(&ask_scroll);
    let right_scroll = gtk4::ScrolledWindow::builder()
        .child(&right)
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .build();

    let paned = gtk4::Paned::new(gtk4::Orientation::Horizontal);
    paned.set_start_child(Some(&left));
    paned.set_end_child(Some(&right_scroll));
    paned.set_resize_end_child(false);
    paned.set_shrink_end_child(false);

    // Past meetings.
    let list = gtk4::ListBox::new();
    let list_scroll = gtk4::ScrolledWindow::builder()
        .child(&list)
        .vexpand(true)
        .build();

    let stack = gtk4::Stack::new();
    stack.add_named(&list_scroll, Some("list"));
    stack.add_named(&paned, Some("meeting"));

    let status = label("");
    status.set_margin_start(8);
    status.set_margin_bottom(4);
    let root = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    root.append(&stack);
    root.append(&status);
    window.set_child(Some(&root));

    // Keep the save and line buttons reachable from callbacks.
    save_notes.set_widget_name("save-notes");
    move_button.set_widget_name("move-line");
    note_button.set_widget_name("note-line");

    Ui {
        window,
        stack,
        state_label,
        title_label,
        elapsed_label,
        back_button,
        pause_button,
        stop_button,
        transcript,
        transcript_scroll,
        follow_button,
        chips,
        notes,
        summary,
        ask_entry,
        ask_answer,
        list,
        status,
    }
}

fn state_markup(state: ViewState) -> &'static str {
    match state {
        ViewState::Recording => "<span foreground=\"#e01b24\"><b>● Recording</b></span>",
        ViewState::Paused => "<span foreground=\"#e5a50a\"><b>❚❚ Paused</b></span>",
        ViewState::Finalizing => "<b>Finalizing…</b>",
        ViewState::Summarizing => "<b>Summarizing…</b>",
        ViewState::Ended => "<b>Ended</b>",
        ViewState::Idle => "<b>No meeting</b>",
    }
}

/// Bring every widget in line with the state.
fn refresh(ui: &Ui, st: &mut State) {
    let on_list = st.stored.is_none()
        && !st.live.state.is_live()
        && st.live.state != ViewState::Summarizing
        && st.live.meeting_id.is_none();
    let showing_list = ui.stack.visible_child_name().as_deref() == Some("list");
    if on_list && !showing_list {
        ui.stack.set_visible_child_name("list");
    }
    let showing_list = ui.stack.visible_child_name().as_deref() == Some("list");

    let view = st.view().clone();
    ui.state_label.set_markup(state_markup(view.state));
    ui.title_label.set_text(if showing_list {
        "Meetings"
    } else {
        view.title.as_deref().unwrap_or("Meeting")
    });
    let elapsed = if showing_list {
        String::new()
    } else {
        view.elapsed(chrono::Utc::now())
            .map(clock)
            .unwrap_or_default()
    };
    let elapsed = if view.live && !view.connected && !showing_list {
        format!("{elapsed}  reconnecting…")
    } else {
        elapsed
    };
    ui.elapsed_label.set_text(&elapsed);
    let live = st.stored.is_none() && st.live.state.is_live();
    ui.pause_button.set_visible(live && !showing_list);
    ui.stop_button.set_visible(live && !showing_list);
    ui.pause_button
        .set_label(if st.live.state == ViewState::Paused {
            "Resume"
        } else {
            "Pause"
        });
    ui.back_button.set_visible(!showing_list);
    ui.follow_button.set_visible(!st.follow);

    let key = (st.stored.is_some(), view.revision, view.meeting_id.clone());
    if st.rendered_revision.as_ref() != Some(&key) {
        st.rendered_revision = Some(key);
        render_transcript(ui, st, &view);
        render_side(ui, st, &view);
    }
    if let Some(a) = &st.ask {
        let who = a.agent_name.clone().unwrap_or_else(|| "Assistant".into());
        let body = match &a.error {
            Some(e) => format!("<span foreground=\"#e01b24\">{}</span>", escape_markup(e)),
            None if a.answer.is_empty() && !a.done => "…".into(),
            None => escape_markup(&a.answer),
        };
        ui.ask_answer.set_markup(&format!(
            "<b>{}</b>\n{}\n<span alpha=\"60%\">{}</span>",
            escape_markup(&a.question),
            body,
            if a.done {
                who
            } else {
                format!("{who} is answering…")
            }
        ));
    }
}

fn render_transcript(ui: &Ui, st: &State, view: &MeetingView) {
    while let Some(child) = ui.transcript.first_child() {
        ui.transcript.remove(&child);
    }
    if view.lines().is_empty() {
        let l = label(if view.state == ViewState::Idle {
            "No meeting is running."
        } else {
            "Listening… lines appear as each chunk is transcribed."
        });
        l.add_css_class("dim-label");
        ui.transcript.append(&l);
        return;
    }
    for seg in view.lines() {
        let l = label("");
        l.set_markup(&line_markup(view, seg));
        l.set_wrap(true);
        l.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
        l.set_margin_top(2);
        l.set_margin_bottom(2);
        l.set_margin_start(6);
        l.set_margin_end(6);
        l.set_widget_name(&seg.id.to_string());
        ui.transcript.append(&l);
    }
    if let Some(sel) = st.selected {
        if let Some(i) = view.lines().iter().position(|s| s.id == sel) {
            if let Some(row) = ui.transcript.row_at_index(i as i32) {
                ui.transcript.select_row(Some(&row));
            }
        }
    }
}

fn render_side(ui: &Ui, st: &mut State, view: &MeetingView) {
    while let Some(child) = ui.chips.first_child() {
        ui.chips.remove(&child);
    }
    for (i, chip) in view.speakers().iter().enumerate() {
        let b = gtk4::Button::new();
        let l = label("");
        l.set_markup(&format!(
            "<span foreground=\"{}\">●</span> {} <span alpha=\"60%\">({})</span>",
            SPEAKER_COLORS[i % SPEAKER_COLORS.len()],
            escape_markup(&chip.name),
            chip.lines
        ));
        b.set_child(Some(&l));
        b.set_tooltip_text(Some("Rename this speaker"));
        b.set_widget_name(&chip.name);
        b.add_css_class("flat");
        let (tx, name) = (st.tx.clone(), chip.name.clone());
        b.connect_clicked(move |_| {
            let _ = tx.send(Msg::Rename(name.clone()));
        });
        ui.chips.append(&b);
    }

    let summary = match (&view.summary, view.state) {
        (Some(s), _) => {
            let mut t = format!(
                "{}\n<span alpha=\"60%\">by {}</span>",
                escape_markup(&s.summary.summary),
                escape_markup(&s.agent_name)
            );
            for item in &s.summary.action_items {
                t.push_str(&format!(
                    "\n☐ {}{}",
                    escape_markup(&item.description),
                    item.assignee
                        .as_ref()
                        .map(|a| format!(" ({})", escape_markup(a)))
                        .unwrap_or_default()
                ));
            }
            t
        }
        (None, ViewState::Summarizing) => "Writing the summary…".into(),
        (None, _) => "<span alpha=\"60%\">No summary yet.</span>".into(),
    };
    let summary = match &view.last_error {
        Some(e) => format!(
            "{summary}\n<span foreground=\"#e01b24\">{}</span>",
            escape_markup(e)
        ),
        None => summary,
    };
    ui.summary.set_markup(&summary);

    // Notes: load when the meeting changes or the daemon says they changed,
    // unless the user is in the middle of editing them.
    let id = view.meeting_id.clone();
    let changed = st.notes_loaded_for != id || st.notes_seen_at != view.notes_updated_at;
    if changed && !st.notes_dirty {
        let notes = id
            .as_deref()
            .and_then(|mid| voxtype::meeting::MeetingId::parse(mid).ok())
            .and_then(|mid| voxtype::meeting::notes::load(&st.meetings, &mid).ok())
            .map(|n| n.notes)
            .unwrap_or_default();
        ui.notes.buffer().set_text(&notes);
        st.notes_dirty = false;
        st.notes_loaded_for = id;
        st.notes_seen_at = view.notes_updated_at;
    }
}

fn render_list(ui: &Ui, st: &State) {
    while let Some(child) = ui.list.first_child() {
        ui.list.remove(&child);
    }
    if st.live.state.is_live() || st.live.state == ViewState::Summarizing {
        let l = label("");
        l.set_markup(&format!(
            "{}  <b>{}</b>",
            state_markup(st.live.state),
            escape_markup(st.live.title.as_deref().unwrap_or("Live meeting"))
        ));
        l.set_widget_name("live");
        l.set_margin_top(6);
        l.set_margin_bottom(6);
        ui.list.append(&l);
    }
    if st.list.is_empty() && !(st.live.state.is_live() || st.live.state == ViewState::Summarizing) {
        let l = label("No meetings yet. Start one with: voxtype meeting start");
        l.set_widget_name("empty");
        l.set_margin_top(6);
        ui.list.append(&l);
    }
    for e in &st.list {
        let m = &e.metadata;
        let l = label("");
        l.set_markup(&format!(
            "<span alpha=\"60%\">{}</span>  <b>{}</b>  {}  {} speaker{}{}",
            m.started_at
                .with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M"),
            escape_markup(&m.display_title()),
            m.duration_secs
                .map(|d| clock(Duration::from_secs(d)))
                .unwrap_or_else(|| "--:--".into()),
            e.speaker_count,
            if e.speaker_count == 1 { "" } else { "s" },
            if e.has_summary { "  · summary" } else { "" }
        ));
        l.set_widget_name(&m.id.to_string());
        l.set_margin_top(6);
        l.set_margin_bottom(6);
        ui.list.append(&l);
    }
}

/// A small modal window with an entry and buttons.
fn prompt(
    parent: &gtk4::ApplicationWindow,
    title: &str,
    choices: &[String],
    placeholder: &str,
    on_done: impl Fn(String) + 'static,
) {
    let win = gtk4::Window::builder()
        .transient_for(parent)
        .modal(true)
        .title(title)
        .default_width(360)
        .build();
    let body = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
    body.set_margin_top(12);
    body.set_margin_bottom(12);
    body.set_margin_start(12);
    body.set_margin_end(12);
    let on_done = Rc::new(on_done);
    for c in choices {
        let b = gtk4::Button::with_label(c);
        let (win, on_done, c) = (win.clone(), on_done.clone(), c.clone());
        b.connect_clicked(move |_| {
            on_done(c.clone());
            win.close();
        });
        body.append(&b);
    }
    let entry = gtk4::Entry::new();
    entry.set_placeholder_text(Some(placeholder));
    body.append(&entry);
    let ok = gtk4::Button::with_label("OK");
    {
        let (win, on_done, input) = (win.clone(), on_done.clone(), entry.clone());
        let submit = move || {
            let text = input.text().trim().to_string();
            if !text.is_empty() {
                on_done(text);
                win.close();
            }
        };
        let s2 = submit.clone();
        ok.connect_clicked(move |_| s2());
        entry.connect_activate(move |_| submit());
    }
    body.append(&ok);
    win.set_child(Some(&body));
    win.present();
}

fn open_meeting(ui: &Ui, st: &mut State, id: Option<&str>) {
    let live_id = st.live.meeting_id.clone();
    match id {
        None => st.stored = None,
        Some(id) if Some(id) == live_id.as_deref() || id == "current" => st.stored = None,
        Some(id) => match load_stored(&st.meetings, id) {
            Some(v) => st.stored = Some(v),
            None => {
                ui.status.set_text(&format!("Could not load meeting {id}"));
                return;
            }
        },
    }
    st.follow = st.stored.is_none();
    st.selected = None;
    st.notes_dirty = false;
    st.rendered_revision = None;
    ui.stack.set_visible_child_name("meeting");
}

fn apply_launch(ui: &Ui, st: &mut State, launch: &Launch) {
    if launch.meeting.is_some()
        || launch.tab.is_some()
        || launch.ask_id.is_some()
        || st.live.state.is_live()
    {
        open_meeting(ui, st, launch.meeting.as_deref());
    }
    if let (Some(ask_id), Some(id)) = (&launch.ask_id, st.meeting_id()) {
        if let Ok(m) = voxtype::meeting::get_meeting(&st.meetings, &id) {
            if let Some(folder) = m.metadata.storage_path.as_deref() {
                if let Some(rec) = voxtype::meeting::assistant::asks::load(folder)
                    .into_iter()
                    .find(|r| &r.id == ask_id)
                {
                    st.ask = Some(AskAnswer {
                        question: rec.question,
                        answer: rec.answer,
                        agent_name: Some(rec.agent_name),
                        done: true,
                        error: None,
                    });
                }
            }
        }
    }
    if let Some(q) = &launch.ask {
        start_ask(st, q);
    }
    if launch.tab.as_deref() == Some("ask") || launch.ask_id.is_some() {
        ui.ask_entry.grab_focus();
    } else if launch.tab.as_deref() == Some("notes") {
        ui.notes.grab_focus();
    }
    st.rendered_revision = None;
    ui.window.present();
}

fn start_ask(st: &mut State, question: &str) {
    let Some(id) = st.meeting_id() else {
        return;
    };
    st.ask = Some(AskAnswer::new(question));
    let (cli, tx, q) = (st.cli.clone(), st.tx.clone(), question.to_string());
    std::thread::spawn(move || {
        let tx2 = tx.clone();
        let ok = cli.ask(&id, &q, |line| {
            let _ = tx2.send(Msg::AskLine(line.to_string()));
        });
        let _ = tx.send(Msg::AskExited(ok));
    });
}

fn selected_segment(ui: &Ui, st: &State) -> Option<u32> {
    let row = ui.transcript.selected_row()?;
    let i = row.index();
    st.view()
        .lines()
        .get(usize::try_from(i).ok()?)
        .map(|s| s.id)
}

fn wire(ui: Rc<Ui>, shared: Shared, feed_rx: Receiver<Feed>, msg_rx: Receiver<Msg>) {
    // Pause / Resume / Stop.
    {
        let s = shared.clone();
        ui.pause_button.connect_clicked(move |_| {
            let st = s.borrow();
            st.run(if st.live.state == ViewState::Paused {
                commands::resume()
            } else {
                commands::pause()
            });
        });
    }
    {
        let (s, u) = (shared.clone(), ui.clone());
        ui.stop_button.connect_clicked(move |_| {
            let s = s.clone();
            prompt(
                &u.window,
                "Stop the meeting?",
                &["Stop meeting".to_string()],
                "Or press Escape to keep recording",
                move |choice| {
                    if choice == "Stop meeting" {
                        s.borrow().run(commands::stop());
                    }
                },
            );
        });
    }
    // Back to the list.
    {
        let (s, u) = (shared.clone(), ui.clone());
        ui.back_button.connect_clicked(move |_| {
            let mut st = s.borrow_mut();
            st.stored = None;
            st.list =
                voxtype::meeting::list_meeting_entries(&st.meetings, Some(50)).unwrap_or_default();
            render_list(&u, &st);
            u.stack.set_visible_child_name("list");
            st.rendered_revision = None;
            refresh(&u, &mut st);
        });
    }
    // Open a meeting from the list.
    {
        let (s, u) = (shared.clone(), ui.clone());
        ui.list.connect_row_activated(move |_, row| {
            let name = row
                .child()
                .map(|c| c.widget_name().to_string())
                .unwrap_or_default();
            let mut st = s.borrow_mut();
            if name == "empty" {
                return;
            }
            let id = (name != "live").then_some(name);
            open_meeting(&u, &mut st, id.as_deref());
            refresh(&u, &mut st);
        });
    }
    // Selecting a line stops following unless it is the newest.
    {
        let s = shared.clone();
        ui.transcript.connect_row_selected(move |list, row| {
            let Ok(mut st) = s.try_borrow_mut() else {
                return;
            };
            let Some(row) = row else {
                return;
            };
            let i = row.index() as usize;
            st.selected = st.view().lines().get(i).map(|l| l.id);
            let last = st.view().lines().len().saturating_sub(1);
            if i < last && list.has_focus() {
                st.follow = false;
            }
        });
    }
    // Scrolling up stops following; new lines scroll into view otherwise.
    {
        let adj = ui.transcript_scroll.vadjustment();
        let s = shared.clone();
        adj.connect_value_changed(move |a| {
            let Ok(mut st) = s.try_borrow_mut() else {
                return;
            };
            if a.value() + a.page_size() < a.upper() - 24.0 {
                st.follow = false;
            }
        });
        let s = shared.clone();
        adj.connect_changed(move |a| {
            if s.try_borrow().map(|st| st.follow).unwrap_or(false) {
                a.set_value(a.upper() - a.page_size());
            }
        });
    }
    {
        let (s, u) = (shared.clone(), ui.clone());
        ui.follow_button.connect_clicked(move |_| {
            let mut st = s.borrow_mut();
            st.follow = true;
            let a = u.transcript_scroll.vadjustment();
            a.set_value(a.upper() - a.page_size());
            refresh(&u, &mut st);
        });
    }
    // Move the selected line / note on it.
    if let Some(move_button) = find_button(&ui.window, "move-line") {
        let (s, u) = (shared.clone(), ui.clone());
        move_button.connect_clicked(move |_| {
            let st = s.borrow();
            let (Some(id), Some(seg)) = (st.meeting_id(), selected_segment(&u, &st)) else {
                u.status.set_text("Select a line first");
                return;
            };
            let names: Vec<String> = st.view().speakers().into_iter().map(|c| c.name).collect();
            let s2 = s.clone();
            prompt(
                &u.window,
                "Move this line to",
                &names,
                "New speaker's name",
                move |who| {
                    s2.borrow().run(commands::reassign(&id, seg, &who));
                },
            );
        });
    }
    if let Some(note_button) = find_button(&ui.window, "note-line") {
        let (s, u) = (shared.clone(), ui.clone());
        note_button.connect_clicked(move |_| {
            let st = s.borrow();
            let (Some(id), Some(seg)) = (st.meeting_id(), selected_segment(&u, &st)) else {
                u.status.set_text("Select a line first");
                return;
            };
            let s2 = s.clone();
            prompt(&u.window, "Note on this line", &[], "Note", move |text| {
                s2.borrow().run(commands::annotate(&id, seg, &text));
            });
        });
    }
    // Notes.
    {
        let s = shared.clone();
        ui.notes.buffer().connect_changed(move |_| {
            if let Ok(mut st) = s.try_borrow_mut() {
                st.notes_dirty = true;
            }
        });
    }
    if let Some(save) = find_button(&ui.window, "save-notes") {
        let (s, u) = (shared.clone(), ui.clone());
        save.connect_clicked(move |_| {
            let mut st = s.borrow_mut();
            let Some(id) = st.meeting_id() else {
                return;
            };
            let buf = u.notes.buffer();
            let text = buf
                .text(&buf.start_iter(), &buf.end_iter(), false)
                .to_string();
            st.notes_dirty = false;
            st.run(commands::set_notes(&id, &text));
        });
    }
    // Ask.
    {
        let (s, u) = (shared.clone(), ui.clone());
        ui.ask_entry.connect_activate(move |e| {
            let q = e.text().trim().to_string();
            if q.is_empty() {
                return;
            }
            e.set_text("");
            let mut st = s.borrow_mut();
            start_ask(&mut st, &q);
            refresh(&u, &mut st);
        });
    }

    // The loop: socket events, worker results, the clock.
    let (s, u) = (shared, ui);
    glib::timeout_add_local(Duration::from_millis(100), move || {
        let mut st = s.borrow_mut();
        let mut resync = false;
        let mut list_changed = false;
        while let Ok(feed) = feed_rx.try_recv() {
            match feed {
                Feed::Connected => {}
                Feed::Disconnected => st.live.disconnected(),
                Feed::Line(line) => {
                    let was_live = st.live.state.is_live();
                    if st.live.apply_line(&line) == Change::Resync {
                        resync = true;
                    }
                    if let Some(stored) = st.stored.as_mut() {
                        stored.apply_line(&line);
                    }
                    if was_live != st.live.state.is_live() {
                        list_changed = true;
                    }
                }
            }
        }
        if resync {
            if let Some(f) = st.feed.as_ref() {
                f.resync();
            }
        }
        while let Ok(msg) = msg_rx.try_recv() {
            match msg {
                Msg::Done(out) => {
                    u.status.set_text(&out.status_line());
                    let ended = st.stored.is_some() || !st.live.state.is_live();
                    if out.ok && out.reload && ended {
                        if let Some(id) = st.meeting_id() {
                            if let Some(v) = load_stored(&st.meetings, &id) {
                                st.stored = Some(v);
                            }
                        }
                    }
                }
                Msg::Rename(name) => {
                    if let Some(id) = st.meeting_id() {
                        let s2 = s.clone();
                        prompt(
                            &u.window,
                            &format!("Rename {name}"),
                            &[],
                            "Name",
                            move |new_name| {
                                s2.borrow().run(commands::label(&id, &name, &new_name));
                            },
                        );
                    }
                }
                Msg::AskLine(line) => {
                    if let Some(a) = st.ask.as_mut() {
                        a.apply_line(&line);
                    }
                }
                Msg::AskExited(ok) => {
                    if let Some(a) = st.ask.as_mut() {
                        a.finished(ok);
                    }
                }
            }
        }
        if list_changed && u.stack.visible_child_name().as_deref() == Some("list") {
            render_list(&u, &st);
        }
        refresh(&u, &mut st);
        glib::ControlFlow::Continue
    });
}

fn find_button(window: &gtk4::ApplicationWindow, name: &str) -> Option<gtk4::Button> {
    fn walk(w: &gtk4::Widget, name: &str) -> Option<gtk4::Button> {
        if w.widget_name() == name {
            if let Ok(b) = w.clone().downcast::<gtk4::Button>() {
                return Some(b);
            }
        }
        let mut child = w.first_child();
        while let Some(c) = child {
            if let Some(b) = walk(&c, name) {
                return Some(b);
            }
            child = c.next_sibling();
        }
        None
    }
    walk(window.upcast_ref(), name)
}

/// The smoke test's view of the window.
fn smoke_report(ui: &Ui, st: &State) -> serde_json::Value {
    let mut rows = Vec::new();
    let mut child = ui.transcript.first_child();
    while let Some(c) = child {
        if let Some(l) = c
            .downcast_ref::<gtk4::ListBoxRow>()
            .and_then(|r| r.child())
            .and_then(|w| w.downcast::<gtk4::Label>().ok())
        {
            rows.push(l.text().to_string());
        }
        child = c.next_sibling();
    }
    let mut chips = Vec::new();
    let mut child = ui.chips.first_child();
    while let Some(c) = child {
        chips.push(c.widget_name().to_string());
        child = c.next_sibling();
    }
    let mut list_rows = 0;
    let mut c = ui.list.first_child();
    while let Some(w) = c {
        list_rows += 1;
        c = w.next_sibling();
    }
    serde_json::json!({
        "screen": ui.stack.visible_child_name().map(|s| s.to_string()),
        "state": ui.state_label.text().to_string(),
        "title": ui.title_label.text().to_string(),
        "connected": st.live.connected,
        "pause_visible": ui.pause_button.is_visible(),
        "rows": rows,
        "chips": chips,
        "summary": ui.summary.text().to_string(),
        "list_rows": list_rows,
    })
}

fn main() -> glib::ExitCode {
    // Before GTK starts: the release check runs this where there is no display.
    if std::env::args().nth(1).as_deref() == Some("--version") {
        println!("voxtype-meetings-gtk4 {}", env!("CARGO_PKG_VERSION"));
        return glib::ExitCode::SUCCESS;
    }
    // The smoke test runs without a session bus, where a unique
    // application cannot register.
    let mut flags = gio::ApplicationFlags::HANDLES_COMMAND_LINE;
    if std::env::var_os(SMOKE_ENV).is_some() {
        flags |= gio::ApplicationFlags::NON_UNIQUE;
    }
    let app = gtk4::Application::builder()
        .application_id(APP_ID)
        .flags(flags)
        .build();
    let ui_cell: Running = Rc::new(RefCell::new(None));

    app.connect_command_line(move |app, cmdline| {
        let args: Vec<String> = cmdline
            .arguments()
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        let launch = parse_launch(&args);
        let mut cell = ui_cell.borrow_mut();
        if let Some((ui, shared)) = cell.as_ref() {
            // A second launch: show what it asked for in this window.
            apply_launch(ui, &mut shared.borrow_mut(), &launch);
            return glib::ExitCode::SUCCESS;
        }

        let ui = Rc::new(build_ui(app));
        let (feed_tx, feed_rx) = mpsc::channel();
        let feed = spawn_feed(voxtype::meeting::events::default_socket_path(), feed_tx);
        let (tx, msg_rx) = mpsc::channel();
        let meetings = meeting_config();
        let state = State {
            list: voxtype::meeting::list_meeting_entries(&meetings, Some(50)).unwrap_or_default(),
            meetings,
            cli: Cli::current().unwrap_or(Cli {
                exe: PathBuf::from("voxtype"),
                global_args: Vec::new(),
            }),
            live: MeetingView::live(),
            stored: None,
            follow: true,
            selected: None,
            rendered_revision: None,
            ask: None,
            notes_dirty: false,
            notes_loaded_for: None,
            notes_seen_at: None,
            tx,
            feed: Some(feed),
        };
        let shared: Shared = Rc::new(RefCell::new(state));
        render_list(&ui, &shared.borrow());
        ui.stack.set_visible_child_name("list");
        wire(ui.clone(), shared.clone(), feed_rx, msg_rx);
        {
            // Let the first hello arrive, then honour the launch.
            let (u, s) = (ui.clone(), shared.clone());
            glib::timeout_add_local_once(Duration::from_millis(400), move || {
                let mut st = s.borrow_mut();
                apply_launch(&u, &mut st, &launch);
                render_list(&u, &st);
                refresh(&u, &mut st);
            });
        }
        if let Some(secs) = std::env::var(SMOKE_ENV)
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
        {
            let (u, s, app) = (ui.clone(), shared.clone(), app.clone());
            glib::timeout_add_local_once(Duration::from_secs(secs), move || {
                println!("{}", smoke_report(&u, &s.borrow()));
                app.quit();
            });
        }
        ui.window.present();
        *cell = Some((ui, shared));
        glib::ExitCode::SUCCESS
    });
    app.run()
}
