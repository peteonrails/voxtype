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
//!
//! The live meeting (and one that has just ended) can be fixed from here:
//! rename speakers, move a line, add a note, edit the notes. A past meeting
//! opened from the list is read-only apart from its title, and shows its
//! `summary.md`. Any meeting can be renamed from its title; one the daemon
//! is done with can be deleted from the list or its header.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use gtk4::prelude::*;
use gtk4::{gio, glib};

#[path = "voxtype_meetings_gtk4/history.rs"]
mod history;

use voxtype::meeting::storage::MeetingListEntry;
use voxtype::meeting::view::client::{spawn_feed, Feed, FeedHandle};
use voxtype::meeting::view::commands::{self, AskAnswer, Cli, Outcome, Request};
use voxtype::meeting::view::{
    clock, escape_markup, line_markup, line_text, meter, summary_markup, Change, MeetingView,
    SpeakerChip, ViewEvent, ViewState, SPEAKER_COLORS,
};
use voxtype::meeting::{MeetingConfig, StorageConfig};

const APP_ID: &str = "io.voxtype.Meetings";

/// Set to a number of seconds to print the window's state as JSON and quit:
/// the headless smoke test drives the real window this way.
const SMOKE_ENV: &str = "VOXTYPE_MEETINGS_GTK4_SMOKE";

/// What a launch asked for.
#[derive(Debug, Default, Clone)]
struct Launch {
    config_path: Option<PathBuf>,
    meeting: Option<String>,
    tab: Option<String>,
    ask_id: Option<String>,
    ask: Option<String>,
}

fn parse_launch(args: &[String], cwd: Option<&Path>) -> Result<Launch, &'static str> {
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
                let p = PathBuf::from(value().ok_or("--config needs a path")?);
                l.config_path = Some(if p.is_absolute() {
                    p
                } else {
                    cwd.ok_or("Cannot resolve relative --config: invocation working directory is unavailable")?.join(p)
                });
            }
            "--version" => {
                println!("voxtype-meetings-gtk4 {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            _ => {}
        }
    }
    Ok(l)
}

/// Messages from worker threads.
enum Msg {
    Done(ViewAuthority, Outcome),
    /// A speaker chip was clicked.
    Rename(ViewAuthority, SpeakerChip),
    /// `meeting label` named these keys; the socket does not say so.
    Labeled(ViewAuthority, Vec<String>, String),
    AskLine(ViewAuthority, u64, String),
    AskExited(ViewAuthority, u64, bool),
    /// A meeting's Delete was clicked in the list: ask first.
    ConfirmDelete {
        id: String,
        title: String,
    },
    /// `meeting delete` ran for this meeting.
    Deleted(String, Outcome),
    /// The transcript as text for the clipboard, or why there is none.
    Copied(Result<String, String>),
}

/// Configuration and effective selection that authorized a meeting action.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ViewAuthority {
    generation: u64,
    view_epoch: u64,
    meeting_id: Option<String>,
}

/// Widgets the refresh code updates.
struct Ui {
    window: gtk4::ApplicationWindow,
    stack: gtk4::Stack,
    state_label: gtk4::Label,
    title_label: gtk4::Label,
    /// The title, clickable to rename the meeting on screen.
    title_button: gtk4::Button,
    title_edit: gtk4::Image,
    elapsed_label: gtk4::Label,
    levels: gtk4::Box,
    mic_level: gtk4::LevelBar,
    call_level: gtk4::LevelBar,
    back_button: gtk4::Button,
    pause_button: gtk4::Button,
    stop_button: gtk4::Button,
    delete_button: gtk4::Button,
    transcript: gtk4::ListBox,
    transcript_scroll: gtk4::ScrolledWindow,
    follow_button: gtk4::Button,
    move_button: gtk4::Button,
    note_button: gtk4::Button,
    copy_button: gtk4::Button,
    /// The meeting's audio: drag it into another app, click for its folder.
    audio_button: gtk4::Button,
    audio_label: gtk4::Label,
    read_only: gtk4::Label,
    chips: gtk4::Box,
    notes: gtk4::TextView,
    save_notes: gtk4::Button,
    summary: gtk4::Label,
    ask_entry: gtk4::Entry,
    ask_answer: gtk4::Label,
    list: gtk4::ListBox,
    start_title: gtk4::Entry,
    start_button: gtk4::Button,
    status: gtk4::Label,
}

/// Everything the window knows.
struct State {
    generation: u64,
    view_epoch: u64,
    // A hello can replace live.meeting_id before selection is reconciled.
    view_id: Option<String>,
    ask_serial: u64,
    snapshot_ready: bool,
    pending_launch: Option<(Instant, Launch)>,
    meetings: MeetingConfig,
    cli: Cli,
    live: MeetingView,
    stored: Option<MeetingView>,
    /// `stored` is a past meeting opened from the list (read-only), not the
    /// meeting that just ended, read again after a change.
    past: bool,
    list: Vec<MeetingListEntry>,
    follow: bool,
    /// Segment id of the selected line.
    selected: Option<u32>,
    /// What the transcript and side panel were last drawn from: stored or
    /// live, the view's revision, its meeting, and the line playing.
    rendered_revision: Option<(bool, u64, Option<String>, Option<u32>)>,
    ask: Option<AskAnswer>,
    notes_dirty: bool,
    notes_loaded_for: Option<String>,
    notes_seen_at: Option<chrono::DateTime<chrono::Utc>>,
    /// The folder and audio of the meeting on screen, and the meeting and
    /// whether it was live when they were looked up.
    files: Option<MeetingFiles>,
    files_for: Option<(String, bool)>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    feed_rx: Receiver<Feed>,
    feed: Option<FeedHandle>,
}

impl State {
    fn new(meetings: MeetingConfig, cli: Cli, generation: u64) -> Self {
        let (tx, rx) = mpsc::channel();
        let (feed_tx, feed_rx) = mpsc::channel();
        let feed = spawn_feed(voxtype::meeting::events::default_socket_path(), feed_tx);
        Self {
            generation,
            view_epoch: 0,
            view_id: None,
            ask_serial: 0,
            snapshot_ready: false,
            pending_launch: None,
            list: voxtype::meeting::list_meeting_entries(&meetings, Some(50)).unwrap_or_default(),
            meetings,
            cli,
            live: MeetingView::live(),
            stored: None,
            past: false,
            follow: true,
            selected: None,
            rendered_revision: None,
            ask: None,
            notes_dirty: false,
            notes_loaded_for: None,
            notes_seen_at: None,
            files: None,
            files_for: None,
            tx,
            rx,
            feed_rx,
            feed: Some(feed),
        }
    }

    fn view(&self) -> &MeetingView {
        self.stored.as_ref().unwrap_or(&self.live)
    }

    fn meeting_id(&self) -> Option<String> {
        self.view().meeting_id.clone()
    }

    fn authority(&self) -> ViewAuthority {
        ViewAuthority {
            generation: self.generation,
            view_epoch: self.view_epoch,
            meeting_id: self.view_id.clone(),
        }
    }

    fn run(&self, request: Request) {
        let cli = self.cli.clone();
        let tx = self.tx.clone();
        let origin = self.authority();
        std::thread::spawn(move || {
            let _ = tx.send(Msg::Done(origin, cli.run(&request)));
        });
    }

    /// Name a speaker chip: one `meeting label` per key behind it, in
    /// order, stopping at the first that fails; then apply it here.
    fn label(&self, id: &str, chip: &SpeakerChip, name: &str) {
        let requests: Vec<Request> = chip
            .keys
            .iter()
            .map(|key| commands::label(id, key, name))
            .collect();
        let (cli, tx) = (self.cli.clone(), self.tx.clone());
        let origin = self.authority();
        let (keys, name) = (chip.keys.clone(), name.to_string());
        std::thread::spawn(move || {
            let mut last = None;
            for r in &requests {
                let out = cli.run(r);
                let ok = out.ok;
                last = Some(out);
                if !ok {
                    break;
                }
            }
            if let Some(out) = last {
                if out.ok {
                    let _ = tx.send(Msg::Labeled(origin.clone(), keys, name));
                }
                let _ = tx.send(Msg::Done(origin, out));
            }
        });
    }

    /// A past meeting opened from the list: shown, not edited.
    fn read_only(&self) -> bool {
        self.past && self.stored.is_some()
    }

    /// The daemon still works on meeting `id`: records it, finishes it or
    /// writes its summary. Deleting it now would pull its folder away.
    fn daemon_busy_with(&self, id: &str) -> bool {
        self.live.meeting_id.as_deref() == Some(id)
            && (self.live.state.is_live() || self.live.state == ViewState::Summarizing)
    }

    /// The meeting on screen can be deleted.
    fn can_delete(&self) -> bool {
        self.meeting_id()
            .is_some_and(|id| !self.daemon_busy_with(&id))
    }

    /// Export the transcript on screen as text; the loop puts it on the
    /// clipboard.
    fn copy_transcript(&self) {
        let Some(id) = self.meeting_id() else {
            return;
        };
        let (cli, tx) = (self.cli.clone(), self.tx.clone());
        std::thread::spawn(move || {
            let _ = tx.send(Msg::Copied(cli.output(&commands::transcript_text(&id))));
        });
    }

    /// Delete meeting `id`; the loop hears how it went.
    fn delete(&self, id: &str) {
        let (cli, tx, id) = (self.cli.clone(), self.tx.clone(), id.to_string());
        std::thread::spawn(move || {
            let out = cli.run(&commands::delete(&id));
            let _ = tx.send(Msg::Deleted(id, out));
        });
    }
}

type Shared = Rc<RefCell<State>>;

/// The window once it exists, for later launches to reuse.
type Running = Rc<RefCell<Option<(Rc<Ui>, Shared)>>>;

fn meeting_config(config_path: Option<&Path>) -> anyhow::Result<MeetingConfig> {
    use anyhow::Context;
    if let Some(path) = config_path {
        std::fs::File::open(path)
            .with_context(|| format!("Cannot read config {}", path.display()))?;
    }
    let config = voxtype::config::load_config(config_path)?;
    let storage_path = if config.meeting.storage_path == "auto" {
        StorageConfig::default_storage_path()
    } else {
        PathBuf::from(&config.meeting.storage_path)
    };
    Ok(MeetingConfig {
        storage: StorageConfig {
            storage_path,
            my_name: config.meeting.my_name.clone(),
            ..StorageConfig::default()
        },
        ..MeetingConfig::default()
    })
}

fn meeting_cli(config_path: Option<&Path>) -> Cli {
    use voxtype::osd::frontend::{OsdProbe, SystemProbe};
    Cli {
        exe: SystemProbe
            .find_voxtype_binary("voxtype")
            .unwrap_or_else(|| PathBuf::from("voxtype")),
        global_args: config_path
            .map(|p| vec!["--config".into(), p.display().to_string()])
            .unwrap_or_default(),
    }
}

fn load_stored(meetings: &MeetingConfig, id: &str) -> Option<MeetingView> {
    voxtype::meeting::get_meeting(meetings, id)
        .ok()
        .map(|m| MeetingView::stored(&m))
}

/// Where a meeting's files are: its folder, and the audio to hand out,
/// both sides mixed when that was kept.
#[derive(Debug, Clone)]
struct MeetingFiles {
    folder: PathBuf,
    audio: Option<PathBuf>,
}

fn meeting_files(meetings: &MeetingConfig, id: &str) -> Option<MeetingFiles> {
    use voxtype::meeting::retained_audio::{retained_files, MIXED};
    let folder = voxtype::meeting::get_meeting(meetings, id)
        .ok()?
        .metadata
        .storage_path?;
    let files = retained_files(&folder);
    let audio = files
        .iter()
        .find(|(stem, _)| *stem == MIXED)
        .or(files.first())
        .map(|(_, name)| folder.join(name));
    Some(MeetingFiles { folder, audio })
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
    let delete_button = gtk4::Button::with_label("Delete…");
    delete_button.add_css_class("destructive-action");
    delete_button.set_tooltip_text(Some("Delete this meeting and its audio"));
    delete_button.set_visible(false);
    header.pack_start(&back_button);
    header.pack_end(&delete_button);
    header.pack_end(&stop_button);
    header.pack_end(&pause_button);
    let title_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
    let state_label = gtk4::Label::new(None);
    let title_label = gtk4::Label::new(None);
    // The title renames the meeting when clicked, as in the Quickshell view.
    let title_edit = gtk4::Image::from_icon_name("document-edit-symbolic");
    let title_inner = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    title_inner.append(&title_label);
    title_inner.append(&title_edit);
    let title_button = gtk4::Button::new();
    title_button.set_child(Some(&title_inner));
    title_button.add_css_class("flat");
    let elapsed_label = gtk4::Label::new(None);
    title_box.append(&state_label);
    title_box.append(&title_button);
    title_box.append(&elapsed_label);
    // Level meters, while the daemon sends levels for a recording meeting.
    let levels = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
    levels.set_valign(gtk4::Align::Center);
    let level = |name: &str| {
        let bar = gtk4::LevelBar::for_interval(0.0, 1.0);
        bar.set_width_request(60);
        bar.set_tooltip_text(Some(name));
        levels.append(&bar);
        bar
    };
    let mic_level = level("Your microphone");
    let call_level = level("The call");
    levels.set_visible(false);
    title_box.append(&levels);
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
    // The same room around its buttons as the header bar gives its own.
    line_bar.set_margin_top(6);
    line_bar.set_margin_bottom(6);
    line_bar.set_margin_start(6);
    line_bar.set_margin_end(6);
    let move_button = gtk4::Button::with_label("Move line to…");
    let note_button = gtk4::Button::with_label("Note on line…");
    let copy_button = gtk4::Button::with_label("Copy transcript");
    copy_button.set_tooltip_text(Some(
        "Copy the whole transcript with times and speakers. Ctrl+C copies the selected line.",
    ));
    let audio_label = label("");
    let audio_inner = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    audio_inner.append(&gtk4::Image::from_icon_name("audio-x-generic-symbolic"));
    audio_inner.append(&audio_label);
    let audio_button = gtk4::Button::new();
    audio_button.set_child(Some(&audio_inner));
    audio_button.set_tooltip_text(Some(
        "Drag the meeting's audio into another app. Click to open the meeting's folder, with each side's track.",
    ));
    audio_button.set_visible(false);
    let follow_button = gtk4::Button::with_label("Follow new lines");
    let read_only = label("Past meeting: read-only");
    read_only.add_css_class("dim-label");
    line_bar.append(&move_button);
    line_bar.append(&note_button);
    line_bar.append(&copy_button);
    line_bar.append(&audio_button);
    line_bar.append(&read_only);
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
    right.append(&heading("Ask"));
    let ask_entry = gtk4::Entry::new();
    ask_entry.set_placeholder_text(Some("Ask about this meeting"));
    right.append(&ask_entry);
    let ask_answer = label("");
    ask_answer.set_wrap(true);
    ask_answer.set_selectable(true);
    let ask_scroll = gtk4::ScrolledWindow::builder()
        .child(&ask_answer)
        .min_content_height(80)
        .build();
    right.append(&ask_scroll);
    // The summary last: once a meeting has one, it is most of the panel.
    right.append(&heading("Summary"));
    let summary = label("");
    summary.set_wrap(true);
    summary.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
    summary.set_selectable(true);
    summary.set_valign(gtk4::Align::Start);
    summary.set_vexpand(true);
    right.append(&summary);
    let right_scroll = gtk4::ScrolledWindow::builder()
        .child(&right)
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .build();

    let paned = gtk4::Paned::new(gtk4::Orientation::Horizontal);
    paned.set_start_child(Some(&left));
    paned.set_end_child(Some(&right_scroll));
    paned.set_resize_end_child(false);
    paned.set_shrink_end_child(false);

    // No meeting: start one, or open a past one.
    let start_bar = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    start_bar.set_margin_top(8);
    start_bar.set_margin_bottom(4);
    start_bar.set_margin_start(8);
    start_bar.set_margin_end(8);
    let start_title = gtk4::Entry::new();
    start_title.set_placeholder_text(Some("Title (optional)"));
    start_title.set_hexpand(true);
    let start_button = gtk4::Button::with_label("Start meeting");
    start_button.add_css_class("suggested-action");
    start_bar.append(&start_title);
    start_bar.append(&start_button);
    let list = gtk4::ListBox::new();
    let list_scroll = gtk4::ScrolledWindow::builder()
        .child(&list)
        .vexpand(true)
        .build();
    let list_page = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    list_page.append(&start_bar);
    let recent = heading("Recent meetings");
    recent.set_margin_start(8);
    list_page.append(&recent);
    list_page.append(&list_scroll);

    let stack = gtk4::Stack::new();
    stack.add_named(&list_page, Some("list"));
    stack.add_named(&paned, Some("meeting"));

    let status = label("");
    status.set_margin_start(8);
    status.set_margin_bottom(4);
    let root = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    root.append(&stack);
    root.append(&status);
    window.set_child(Some(&root));

    Ui {
        window,
        stack,
        state_label,
        title_label,
        title_button,
        title_edit,
        elapsed_label,
        levels,
        mic_level,
        call_level,
        back_button,
        pause_button,
        stop_button,
        delete_button,
        transcript,
        transcript_scroll,
        follow_button,
        move_button,
        note_button,
        copy_button,
        audio_button,
        audio_label,
        read_only,
        chips,
        notes,
        save_notes,
        summary,
        ask_entry,
        ask_answer,
        list,
        start_title,
        start_button,
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
    if showing_list && st.live.soundcheck {
        ui.state_label
            .set_markup("<span foreground=\"#1c71d8\"><b>Sound check</b></span>");
    } else {
        ui.state_label.set_markup(state_markup(view.state));
    }
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
    let mut elapsed = if view.live && !view.connected && !showing_list {
        format!("{elapsed}  reconnecting…")
    } else {
        elapsed
    };
    if let Some(p) = view.progress.as_ref().filter(|p| p.running()) {
        let what = if p.operation == "import" {
            "importing"
        } else {
            "transcribing again"
        };
        elapsed.push_str(&format!("  {what} {:.0}%", p.fraction() * 100.0));
    }
    ui.elapsed_label.set_text(&elapsed);
    let now = chrono::Utc::now();
    // The meeting's levels, or a sound check's on the list.
    match view
        .current_levels(now)
        .filter(|l| !showing_list || l.soundcheck)
    {
        Some(l) => {
            ui.mic_level.set_value(meter(l.mic));
            ui.call_level.set_value(meter(l.loopback));
            ui.call_level.set_visible(l.loopback.is_some());
            ui.levels.set_visible(true);
        }
        None => ui.levels.set_visible(false),
    }
    let read_only = st.read_only();
    ui.move_button.set_visible(!read_only);
    ui.note_button.set_visible(!read_only);
    ui.read_only.set_visible(read_only);
    ui.save_notes.set_visible(!read_only);
    ui.notes.set_editable(!read_only);
    ui.transcript.set_selection_mode(if read_only {
        gtk4::SelectionMode::None
    } else {
        gtk4::SelectionMode::Single
    });
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
    // A meeting on screen can be renamed, a past or ended one also deleted.
    // On the list the title just says "Meetings".
    let renamable = !showing_list && st.meeting_id().is_some();
    ui.title_button.set_can_target(renamable);
    ui.title_button.set_focusable(renamable);
    ui.title_button
        .set_tooltip_text(renamable.then_some("Rename this meeting"));
    ui.title_edit.set_visible(renamable);
    ui.delete_button
        .set_visible(!showing_list && st.can_delete());
    ui.copy_button
        .set_visible(!showing_list && st.meeting_id().is_some() && !view.lines().is_empty());
    // The audio once the meeting is over: while it records, the file grows.
    let files_key = st
        .meeting_id()
        .filter(|_| !showing_list)
        .map(|id| (id, view.state.is_live()));
    if st.files_for != files_key {
        st.files = match &files_key {
            Some((id, false)) => meeting_files(&st.meetings, id),
            _ => None,
        };
        st.files_for = files_key;
    }
    match st.files.as_ref().and_then(|f| f.audio.as_ref()) {
        Some(audio) => {
            let name = audio.file_name().unwrap_or_default().to_string_lossy();
            ui.audio_label.set_text(&name);
            ui.audio_button.set_visible(true);
        }
        None => ui.audio_button.set_visible(false),
    }
    // Following only means something while lines still arrive.
    ui.follow_button
        .set_visible(!st.follow && st.stored.is_none() && view.state.is_live());
    ui.start_button
        .set_sensitive(st.live.connected && !st.live.state.is_live());
    ui.start_button.set_tooltip_text(if st.live.connected {
        None
    } else {
        Some("The voxtype daemon is not running")
    });

    let key = (
        st.stored.is_some(),
        view.revision,
        view.meeting_id.clone(),
        view.playing_line(now),
    );
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
    } else {
        ui.ask_answer.set_text("");
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
    let playing = view.playing_line(chrono::Utc::now());
    for seg in view.lines() {
        let l = label("");
        l.set_markup(&line_markup(view, seg, playing == Some(seg.id)));
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
    let read_only = st.read_only();
    for (i, chip) in view.speakers().iter().enumerate() {
        let l = label("");
        l.set_markup(&format!(
            "<span foreground=\"{}\">●</span> {} <span alpha=\"60%\">({})</span>",
            SPEAKER_COLORS[i % SPEAKER_COLORS.len()],
            escape_markup(&chip.name),
            chip.lines
        ));
        if read_only {
            l.set_widget_name(&chip.name);
            l.set_margin_start(10);
            ui.chips.append(&l);
            continue;
        }
        let b = gtk4::Button::new();
        b.set_child(Some(&l));
        b.set_tooltip_text(Some("Rename this speaker"));
        b.set_widget_name(&chip.name);
        b.add_css_class("flat");
        let (tx, chip) = (st.tx.clone(), chip.clone());
        let origin = st.authority();
        b.connect_clicked(move |_| {
            let _ = tx.send(Msg::Rename(origin.clone(), chip.clone()));
        });
        ui.chips.append(&b);
    }

    let summary = match (&view.summary, view.state) {
        // The summary as saved, when its file is there.
        (Some(s), _) if s.markdown().is_some() => format!(
            "{}\n\n<span alpha=\"60%\">by {}</span>",
            summary_markup(&s.markdown().unwrap_or_default()),
            escape_markup(&s.agent_name)
        ),
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
        l.set_margin_start(8);
        l.set_margin_bottom(6);
        ui.list.append(&l);
    }
    if st.list.is_empty() && !(st.live.state.is_live() || st.live.state == ViewState::Summarizing) {
        let l = label("No meetings yet.");
        l.add_css_class("dim-label");
        l.set_widget_name("empty");
        l.set_margin_top(6);
        l.set_margin_start(8);
        ui.list.append(&l);
        if let Some(row) = ui.list.row_at_index(0) {
            row.set_activatable(false);
            row.set_selectable(false);
        }
    }
    for e in &st.list {
        let m = &e.metadata;
        let id = m.id.to_string();
        let l = label("");
        l.set_hexpand(true);
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
        l.set_margin_top(6);
        l.set_margin_start(8);
        l.set_margin_bottom(6);
        // The row's name is the meeting's ID: activating the row opens it.
        let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        row.set_widget_name(&id);
        row.append(&l);
        if !st.daemon_busy_with(&id) {
            let delete = gtk4::Button::from_icon_name("user-trash-symbolic");
            delete.add_css_class("flat");
            delete.set_valign(gtk4::Align::Center);
            delete.set_margin_end(8);
            delete.set_tooltip_text(Some("Delete this meeting"));
            delete.update_property(&[gtk4::accessible::Property::Label("Delete this meeting")]);
            let (tx, title) = (st.tx.clone(), m.display_title());
            delete.connect_clicked(move |_| {
                let _ = tx.send(Msg::ConfirmDelete {
                    id: id.clone(),
                    title: title.clone(),
                });
            });
            row.append(&delete);
        }
        ui.list.append(&row);
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
    prompt_with(parent, title, choices, placeholder, "", on_done);
}

/// `prompt`, its entry starting out with `text`, selected.
fn prompt_with(
    parent: &gtk4::ApplicationWindow,
    title: &str,
    choices: &[String],
    placeholder: &str,
    text: &str,
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
    entry.set_text(text);
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
    if !text.is_empty() {
        entry.grab_focus();
        entry.select_region(0, -1);
    }
}

/// A small modal window that asks before something that cannot be undone.
/// Cancel has the focus, so Enter does not confirm; Escape cancels.
fn confirm(
    parent: &gtk4::ApplicationWindow,
    title: &str,
    message: &str,
    action: &str,
    on_confirm: impl Fn() + 'static,
) {
    let win = gtk4::Window::builder()
        .transient_for(parent)
        .modal(true)
        .title(title)
        .default_width(360)
        .build();
    let body = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    body.set_margin_top(12);
    body.set_margin_bottom(12);
    body.set_margin_start(12);
    body.set_margin_end(12);
    let text = label(message);
    text.set_wrap(true);
    text.set_max_width_chars(48);
    body.append(&text);
    let buttons = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    buttons.set_halign(gtk4::Align::End);
    let cancel = gtk4::Button::with_label("Cancel");
    let ok = gtk4::Button::with_label(action);
    ok.add_css_class("destructive-action");
    {
        let win = win.clone();
        cancel.connect_clicked(move |_| win.close());
    }
    {
        let win = win.clone();
        ok.connect_clicked(move |_| {
            on_confirm();
            win.close();
        });
    }
    buttons.append(&cancel);
    buttons.append(&ok);
    body.append(&buttons);
    let keys = gtk4::EventControllerKey::new();
    {
        let win = win.clone();
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gtk4::gdk::Key::Escape {
                win.close();
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
    }
    win.add_controller(keys);
    win.set_child(Some(&body));
    win.present();
    cancel.grab_focus();
}

/// Ask before deleting meeting `id`, then delete it.
fn confirm_delete(ui: &Rc<Ui>, shared: &Shared, id: String, title: String) {
    let (s, u) = (shared.clone(), ui.clone());
    confirm(
        &ui.window,
        "Delete meeting?",
        &format!(
            "This deletes \"{title}\" for good: its transcript, notes, summary and kept audio."
        ),
        "Delete",
        move || {
            let st = s.borrow();
            if st.daemon_busy_with(&id) {
                u.status
                    .set_text("This meeting is still being recorded or summarized.");
                return;
            }
            st.delete(&id);
        },
    );
}

/// Back to the list of meetings, read again.
fn show_list(ui: &Ui, st: &mut State) {
    st.stored = None;
    select_view(ui, st, None);
    st.list = voxtype::meeting::list_meeting_entries(&st.meetings, Some(50)).unwrap_or_default();
    render_list(ui, st);
    ui.stack.set_visible_child_name("list");
    st.rendered_revision = None;
}

fn select_view(ui: &Ui, st: &mut State, id: Option<String>) {
    if st.view_id != id {
        st.view_epoch = st.view_epoch.wrapping_add(1);
        st.view_id = id;
        st.ask_serial = st.ask_serial.wrapping_add(1);
        st.ask = None;
        st.selected = None;
        st.notes_dirty = false;
        st.notes_loaded_for = None;
        st.notes_seen_at = None;
        st.rendered_revision = None;
        ui.ask_entry.set_text("");
        ui.ask_answer.set_text("");
        ui.status.set_text("");
    }
}

fn dialog_current(ui: &Ui, st: &State, origin: &ViewAuthority) -> bool {
    view_current(ui, st, origin) && !st.read_only()
}

/// The meeting a dialog was opened for is still on screen, read-only or
/// not: a past meeting's title can be changed too.
fn view_current(ui: &Ui, st: &State, origin: &ViewAuthority) -> bool {
    let current = st.authority() == *origin;
    if !current {
        ui.status
            .set_text("Meeting selection changed. Reopen this action in the current meeting.");
    }
    current
}

fn open_meeting(ui: &Ui, st: &mut State, id: Option<&str>) {
    let live_id = st.live.meeting_id.clone();
    st.past = false;
    match id {
        None => st.stored = None,
        Some(id) if Some(id) == live_id.as_deref() || id == "current" => st.stored = None,
        Some(id) => match load_stored(&st.meetings, id) {
            Some(v) => {
                st.past = true;
                st.stored = Some(v);
            }
            None => {
                ui.status.set_text(&format!("Could not load meeting {id}"));
                return;
            }
        },
    }
    let selected_id = st.meeting_id();
    select_view(ui, st, selected_id);
    st.follow = st.stored.is_none();
    st.selected = None;
    st.notes_dirty = false;
    st.rendered_revision = None;
    ui.stack.set_visible_child_name("meeting");
}

fn apply_launch(ui: &Ui, st: &mut State, launch: &Launch) {
    let implicit = launch.meeting.is_none() || launch.meeting.as_deref() == Some("current");
    if implicit
        && !st.snapshot_ready
        && (launch.ask.is_some() || launch.ask_id.is_some() || launch.tab.is_some())
    {
        // Identity belongs to this context's initial snapshot, not the old
        // window or a guessed stored ID. Replacing State drops this intent.
        st.pending_launch = Some((Instant::now() + Duration::from_secs(3), launch.clone()));
        ui.status
            .set_text("Waiting for the current meeting status…");
        ui.window.present();
        return;
    }
    apply_resolved_launch(ui, st, launch);
}

fn apply_resolved_launch(ui: &Ui, st: &mut State, launch: &Launch) {
    ui.status.set_text("");
    if launch.meeting.is_some()
        || launch.tab.is_some()
        || launch.ask_id.is_some()
        || st.live.state.is_live()
    {
        open_meeting(ui, st, launch.meeting.as_deref());
    }
    let implicit = launch.meeting.is_none() || launch.meeting.as_deref() == Some("current");
    if implicit
        && st.meeting_id().is_none()
        && (launch.ask.is_some() || launch.ask_id.is_some() || launch.tab.is_some())
    {
        ui.status.set_text(if st.snapshot_ready {
            "No active meeting is available. Open a saved meeting to use this view."
        } else {
            "Meeting status is unavailable. Reconnect to the daemon or open a saved meeting."
        });
        ui.window.present();
        return;
    }
    if let (Some(ask_id), Some(id)) = (&launch.ask_id, st.meeting_id()) {
        let mut found = false;
        if let Ok(m) = voxtype::meeting::get_meeting(&st.meetings, &id) {
            if let Some(folder) = m.metadata.storage_path.as_deref() {
                if let Some(rec) = voxtype::meeting::assistant::asks::load(folder)
                    .into_iter()
                    .find(|r| &r.id == ask_id)
                {
                    found = true;
                    st.ask_serial = st.ask_serial.wrapping_add(1);
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
        if !found {
            ui.status.set_text(&format!(
                "Could not find saved question {ask_id} for meeting {id}"
            ));
        }
    }
    if let Some(q) = &launch.ask {
        if st.meeting_id().is_some() {
            ui.status.set_text("");
            start_ask(st, q);
        } else {
            ui.status.set_text(if st.snapshot_ready {
                "No active meeting is available. Open a saved meeting before asking a question."
            } else {
                "Meeting status is unavailable. Reconnect to the daemon or open a saved meeting."
            });
        }
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
    st.ask_serial = st.ask_serial.wrapping_add(1);
    let (origin, serial) = (st.authority(), st.ask_serial);
    let (cli, tx, q) = (st.cli.clone(), st.tx.clone(), question.to_string());
    std::thread::spawn(move || {
        let tx2 = tx.clone();
        let ok = cli.ask(&id, &q, |line| {
            let _ = tx2.send(Msg::AskLine(origin.clone(), serial, line.to_string()));
        });
        let _ = tx.send(Msg::AskExited(origin, serial, ok));
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

fn wire(ui: Rc<Ui>, shared: Shared) {
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
            let origin = s.borrow().authority();
            let u2 = u.clone();
            prompt(
                &u.window,
                "Stop the meeting?",
                &["Stop meeting".to_string()],
                "Or press Escape to keep recording",
                move |choice| {
                    let st = s.borrow();
                    if choice == "Stop meeting" && dialog_current(&u2, &st, &origin) {
                        st.run(commands::stop());
                    }
                },
            );
        });
    }
    // Start a meeting from the list.
    {
        let (s, u) = (shared.clone(), ui.clone());
        let start = move || {
            let title = u.start_title.text().to_string();
            u.start_title.set_text("");
            s.borrow().run(commands::start(&title));
        };
        let start2 = start.clone();
        ui.start_button.connect_clicked(move |_| start2());
        ui.start_title.connect_activate(move |_| start());
    }
    // Back to the list.
    {
        let (s, u) = (shared.clone(), ui.clone());
        ui.back_button.connect_clicked(move |_| {
            let mut st = s.borrow_mut();
            show_list(&u, &mut st);
            refresh(&u, &mut st);
        });
    }
    // Rename the meeting on screen: click its title.
    {
        let (s, u) = (shared.clone(), ui.clone());
        ui.title_button.connect_clicked(move |_| {
            if u.stack.visible_child_name().as_deref() != Some("meeting") {
                return;
            }
            let st = s.borrow();
            let Some(id) = st.meeting_id() else {
                return;
            };
            let current = st.view().title.clone().unwrap_or_default();
            let origin = st.authority();
            let (s2, u2, before) = (s.clone(), u.clone(), current.clone());
            prompt_with(
                &u.window,
                "Rename meeting",
                &[],
                "Title",
                &current,
                move |title| {
                    let st = s2.borrow();
                    if title != before && view_current(&u2, &st, &origin) {
                        st.run(commands::title(&id, &title));
                    }
                },
            );
        });
    }
    // Copy the transcript, or with Ctrl+C the selected line.
    {
        let s = shared.clone();
        ui.copy_button
            .connect_clicked(move |_| s.borrow().copy_transcript());
    }
    {
        let (s, u) = (shared.clone(), ui.clone());
        let copy_line = gtk4::CallbackAction::new(move |_, _| {
            let st = s.borrow();
            let Some(line) = selected_segment(&u, &st)
                .and_then(|id| st.view().lines().iter().find(|l| l.id == id).cloned())
            else {
                return glib::Propagation::Proceed;
            };
            u.window.clipboard().set_text(&line_text(&line));
            u.status.set_text("Line copied");
            glib::Propagation::Stop
        });
        let shortcuts = gtk4::ShortcutController::new();
        shortcuts.add_shortcut(gtk4::Shortcut::new(
            gtk4::ShortcutTrigger::parse_string("<Control>c"),
            Some(copy_line),
        ));
        ui.transcript.add_controller(shortcuts);
    }
    // The audio: drag it out as a file; a click opens the meeting's folder.
    {
        let drag = gtk4::DragSource::new();
        drag.set_actions(gtk4::gdk::DragAction::COPY);
        let s = shared.clone();
        drag.connect_prepare(move |_, _, _| {
            let audio = s.try_borrow().ok()?.files.as_ref()?.audio.clone()?;
            let file = gio::File::for_path(audio);
            Some(gtk4::gdk::ContentProvider::for_value(&file.to_value()))
        });
        let button = ui.audio_button.clone();
        drag.connect_drag_begin(move |source, _| {
            source.set_icon(Some(&gtk4::WidgetPaintable::new(Some(&button))), 0, 0);
        });
        ui.audio_button.add_controller(drag);
    }
    {
        let (s, u) = (shared.clone(), ui.clone());
        ui.audio_button.connect_clicked(move |_| {
            let Some(folder) = s.borrow().files.as_ref().map(|f| f.folder.clone()) else {
                return;
            };
            let uri = gio::File::for_path(&folder).uri();
            if let Err(e) =
                gio::AppInfo::launch_default_for_uri(&uri, None::<&gio::AppLaunchContext>)
            {
                u.status
                    .set_text(&format!("Could not open {}: {e}", folder.display()));
            }
        });
    }
    // Delete the meeting on screen, once the daemon is done with it.
    {
        let (s, u) = (shared.clone(), ui.clone());
        ui.delete_button.connect_clicked(move |_| {
            let (id, title) = {
                let st = s.borrow();
                let Some(id) = st.meeting_id().filter(|_| st.can_delete()) else {
                    return;
                };
                let title = st
                    .view()
                    .title
                    .clone()
                    .unwrap_or_else(|| "this meeting".into());
                (id, title)
            };
            confirm_delete(&u, &s, id, title);
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
    {
        let (s, u) = (shared.clone(), ui.clone());
        ui.move_button.connect_clicked(move |_| {
            let st = s.borrow();
            let (Some(id), Some(seg)) = (st.meeting_id(), selected_segment(&u, &st)) else {
                u.status.set_text("Select a line first");
                return;
            };
            let names: Vec<String> = st.view().speakers().into_iter().map(|c| c.name).collect();
            let origin = st.authority();
            let u2 = u.clone();
            let s2 = s.clone();
            prompt(
                &u.window,
                "Move this line to",
                &names,
                "New speaker's name",
                move |who| {
                    let st = s2.borrow();
                    if dialog_current(&u2, &st, &origin) {
                        st.run(commands::reassign(&id, seg, &who));
                    }
                },
            );
        });
    }
    {
        let (s, u) = (shared.clone(), ui.clone());
        ui.note_button.connect_clicked(move |_| {
            let st = s.borrow();
            let (Some(id), Some(seg)) = (st.meeting_id(), selected_segment(&u, &st)) else {
                u.status.set_text("Select a line first");
                return;
            };
            let s2 = s.clone();
            let origin = st.authority();
            let u2 = u.clone();
            prompt(&u.window, "Note on this line", &[], "Note", move |text| {
                let st = s2.borrow();
                if dialog_current(&u2, &st, &origin) {
                    st.run(commands::annotate(&id, seg, &text));
                }
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
    {
        let (s, u) = (shared.clone(), ui.clone());
        ui.save_notes.connect_clicked(move |_| {
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
        let mut started = false;
        while let Ok(feed) = st.feed_rx.try_recv() {
            match feed {
                Feed::Connected => {}
                Feed::Disconnected => st.live.disconnected(),
                Feed::Line(line) => {
                    let was_live = st.live.state.is_live();
                    let snapshot = matches!(ViewEvent::parse(&line), Some(ViewEvent::Hello { .. }));
                    let previous_id = snapshot.then(|| st.live.meeting_id.clone()).flatten();
                    let initial_snapshot = snapshot && !st.snapshot_ready;
                    st.snapshot_ready |= snapshot;
                    if st.live.apply_line(&line) == Change::Resync {
                        resync = true;
                    }
                    if let Some(stored) = st.stored.as_mut() {
                        stored.apply_line(&line);
                    }
                    if was_live != st.live.state.is_live() {
                        list_changed = true;
                    }
                    let preserve_saved = initial_snapshot
                        && st
                            .stored
                            .as_ref()
                            .is_some_and(|stored| stored.meeting_id != st.live.meeting_id);
                    let changed_identity = snapshot && previous_id != st.live.meeting_id;
                    if st.live.state.is_live() && (!was_live || changed_identity) && !preserve_saved
                    {
                        started = true;
                    }
                }
            }
        }
        // A meeting that starts takes over the window, as in the Quickshell
        // view, whatever was on screen.
        if started {
            open_meeting(&u, &mut st, None);
        } else if u.stack.visible_child_name().as_deref() == Some("meeting") {
            let id = st.meeting_id();
            select_view(&u, &mut st, id);
        }
        if st
            .pending_launch
            .as_ref()
            .is_some_and(|(deadline, _)| st.snapshot_ready || Instant::now() >= *deadline)
        {
            let (_, launch) = st.pending_launch.take().unwrap();
            apply_resolved_launch(&u, &mut st, &launch);
        }
        if resync {
            if let Some(f) = st.feed.as_ref() {
                f.resync();
            }
        }
        while let Ok(msg) = st.rx.try_recv() {
            match msg {
                Msg::Done(origin, out) if origin == st.authority() => {
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
                Msg::Rename(origin, chip) => {
                    if !dialog_current(&u, &st, &origin) {
                        continue;
                    }
                    if st.meeting_id().is_some() && !st.read_only() {
                        let s2 = s.clone();
                        let u2 = u.clone();
                        prompt(
                            &u.window,
                            &format!("Rename {}", chip.name),
                            &[],
                            "Name",
                            move |new_name| {
                                let st = s2.borrow();
                                if dialog_current(&u2, &st, &origin) {
                                    if let Some(id) = origin.meeting_id.as_deref() {
                                        st.label(id, &chip, &new_name);
                                    }
                                }
                            },
                        );
                    }
                }
                Msg::Labeled(origin, keys, name) if origin == st.authority() => {
                    st.live.apply_label(&keys, &name);
                }
                Msg::AskLine(origin, serial, line)
                    if origin == st.authority() && serial == st.ask_serial =>
                {
                    if let Some(a) = st.ask.as_mut() {
                        a.apply_line(&line);
                    }
                }
                Msg::AskExited(origin, serial, ok)
                    if origin == st.authority() && serial == st.ask_serial =>
                {
                    if let Some(a) = st.ask.as_mut() {
                        a.finished(ok);
                    }
                }
                Msg::ConfirmDelete { id, title } => {
                    if !st.daemon_busy_with(&id) {
                        confirm_delete(&u, &s, id, title);
                    }
                }
                Msg::Deleted(id, out) => {
                    if out.ok {
                        if st.meeting_id().as_deref() == Some(id.as_str()) {
                            show_list(&u, &mut st);
                        } else {
                            st.list =
                                voxtype::meeting::list_meeting_entries(&st.meetings, Some(50))
                                    .unwrap_or_default();
                            list_changed = true;
                        }
                    }
                    u.status.set_text(&out.status_line());
                }
                Msg::Copied(Ok(text)) => {
                    u.window.clipboard().set_text(text.trim_end());
                    u.status.set_text("Transcript copied");
                }
                Msg::Copied(Err(e)) => {
                    u.status.set_text(&format!("Copy transcript failed: {e}"));
                }
                _ => {}
            }
        }
        if list_changed && u.stack.visible_child_name().as_deref() == Some("list") {
            render_list(&u, &st);
        }
        refresh(&u, &mut st);
        glib::ControlFlow::Continue
    });
}

/// With the smoke test, also save the window as a PNG here, to look at.
const SHOT_ENV: &str = "VOXTYPE_MEETINGS_GTK4_SHOT";

/// Draw the window into a PNG.
fn save_png(window: &gtk4::ApplicationWindow, path: &std::path::Path) {
    let (w, h) = (window.width(), window.height());
    let paintable = gtk4::WidgetPaintable::new(Some(window));
    let snapshot = gtk4::Snapshot::new();
    paintable.snapshot(&snapshot, w as f64, h as f64);
    let (Some(node), Some(native)) = (snapshot.to_node(), window.native()) else {
        eprintln!("could not save {}: nothing drawn", path.display());
        return;
    };
    let Some(renderer) = native.renderer() else {
        eprintln!("could not save {}: no renderer", path.display());
        return;
    };
    let texture = renderer.render_texture(node, None);
    if let Err(e) = texture.save_to_png(path) {
        eprintln!("could not save {}: {e}", path.display());
    }
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
    let mut list_delete_buttons = 0;
    let mut c = ui.list.first_child();
    while let Some(w) = c {
        list_rows += 1;
        if w.downcast_ref::<gtk4::ListBoxRow>()
            .and_then(|r| r.child())
            .and_then(|row| row.last_child())
            .is_some_and(|last| last.is::<gtk4::Button>())
        {
            list_delete_buttons += 1;
        }
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
        "list_delete_buttons": list_delete_buttons,
        "rename_enabled": ui.title_button.can_target(),
        "delete_visible": ui.delete_button.is_visible(),
        "copy_visible": ui.copy_button.is_visible(),
        "audio_visible": ui.audio_button.is_visible(),
        "audio_file": ui.audio_label.text().to_string(),
        "start_sensitive": ui.start_button.is_sensitive(),
        "follow_visible": ui.follow_button.is_visible(),
        "read_only": ui.read_only.is_visible(),
        "move_visible": ui.move_button.is_visible(),
        "notes_editable": ui.notes.is_editable(),
        "chip_buttons": ui.chips.first_child().is_some_and(|c| c.is::<gtk4::Button>()),
        "levels_visible": ui.levels.is_visible(),
        "mic_level": ui.mic_level.value(),
        "call_level": ui.call_level.value(),
        "elapsed": ui.elapsed_label.text().to_string(),
    })
}

fn launch_error(
    cmdline: &gio::ApplicationCommandLine,
    error: impl std::fmt::Display,
) -> glib::ExitCode {
    use glib::translate::ToGlibPtr;
    let message = std::ffi::CString::new(format!("{error}\n").replace('\0', "\\0")).unwrap();
    // The literal Rust binding needs GLib 2.80. This ABI is available on our
    // older supported GLib versions and sends errors to the forwarded caller.
    unsafe {
        gio::ffi::g_application_command_line_printerr(
            cmdline.to_glib_none().0,
            c"%s".as_ptr(),
            message.as_ptr(),
        );
    }
    glib::ExitCode::FAILURE
}

fn main() -> glib::ExitCode {
    if std::env::args().any(|arg| arg == "--history") {
        return history::run();
    }
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
        let launch = match parse_launch(&args, cmdline.cwd().as_deref()) {
            Ok(launch) => launch,
            Err(error) => return launch_error(cmdline, error),
        };
        let meetings = match meeting_config(launch.config_path.as_deref()) {
            Ok(meetings) => meetings,
            Err(error) => return launch_error(cmdline, error),
        };
        let cli = meeting_cli(launch.config_path.as_deref());
        let mut cell = ui_cell.borrow_mut();
        if let Some((ui, shared)) = cell.as_ref() {
            // Replace both authority and mailboxes. Old workers keep their
            // original client, but cannot reply into a different context.
            let mut st = shared.borrow_mut();
            let generation = st.generation + 1;
            *st = State::new(meetings, cli, generation);
            ui.ask_answer.set_text("");
            ui.ask_entry.set_text("");
            ui.start_title.set_text("");
            ui.status.set_text("");
            render_list(ui, &st);
            ui.stack.set_visible_child_name("list");
            apply_launch(ui, &mut st, &launch);
            refresh(ui, &mut st);
            return glib::ExitCode::SUCCESS;
        }

        let ui = Rc::new(build_ui(app));
        let state = State::new(meetings, cli, 0);
        let shared: Shared = Rc::new(RefCell::new(state));
        render_list(&ui, &shared.borrow());
        ui.stack.set_visible_child_name("list");
        wire(ui.clone(), shared.clone());
        {
            // Let the first hello arrive, then honour the launch.
            let (u, s) = (ui.clone(), shared.clone());
            glib::timeout_add_local_once(Duration::from_millis(400), move || {
                let mut st = s.borrow_mut();
                if st.generation != 0 {
                    return;
                }
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
                let Some(path) = std::env::var_os(SHOT_ENV) else {
                    app.quit();
                    return;
                };
                // Draw it once it has been painted, so it is laid out.
                let Some(clock) = u.window.frame_clock() else {
                    app.quit();
                    return;
                };
                let (w, app) = (u.window.clone(), app.clone());
                let done = std::cell::Cell::new(false);
                clock.connect_after_paint(move |_| {
                    if !done.replace(true) {
                        save_png(&w, std::path::Path::new(&path));
                        app.quit();
                    }
                });
                u.window.queue_draw();
            });
        }
        ui.window.present();
        *cell = Some((ui, shared));
        glib::ExitCode::SUCCESS
    });
    app.run()
}

#[cfg(test)]
mod routing_tests {
    use super::*;

    #[test]
    fn relative_config_uses_each_invocations_cwd_without_global_state() {
        let args = vec!["gtk".into(), "--config".into(), "config.toml".into()];
        for root in ["/private/A", "/private/B"] {
            let launch = parse_launch(&args, Some(Path::new(root))).unwrap();
            assert_eq!(
                launch.config_path,
                Some(Path::new(root).join("config.toml"))
            );
        }
        assert!(parse_launch(&args, None).is_err());
        assert!(parse_launch(&["gtk".into()], None)
            .unwrap()
            .config_path
            .is_none());
    }

    #[test]
    fn client_is_canonical_and_keeps_explicit_or_default_config() {
        let client = meeting_cli(Some(Path::new("/private/A/config.toml")));
        assert_eq!(client.exe.file_name().unwrap(), "voxtype");
        assert_eq!(client.global_args, ["--config", "/private/A/config.toml"]);
        assert!(meeting_cli(None).global_args.is_empty());
    }

    #[test]
    fn replacing_context_disconnects_old_worker_and_widget_mailboxes() {
        let root = tempfile::tempdir().unwrap();
        let meetings = MeetingConfig {
            storage: StorageConfig {
                storage_path: root.path().join("meetings"),
                ..StorageConfig::default()
            },
            ..MeetingConfig::default()
        };
        let mut state = State::new(
            meetings.clone(),
            meeting_cli(Some(Path::new("/private/A.toml"))),
            0,
        );
        let old_tx = state.tx.clone();
        let old_client = state.cli.clone();
        let old_authority = state.authority();
        state = State::new(meetings, meeting_cli(Some(Path::new("/private/B.toml"))), 1);
        assert!(old_tx
            .send(Msg::AskLine(
                old_authority,
                0,
                "obsolete private A answer".into(),
            ))
            .is_err());
        assert!(matches!(
            state.rx.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
        assert_eq!(old_client.global_args, ["--config", "/private/A.toml"]);
        assert_eq!(state.cli.global_args, ["--config", "/private/B.toml"]);
        assert_eq!(state.generation, 1);
    }
}
