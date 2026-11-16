//! Private-file history in the shipped normal GTK window helper.
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use gtk4::prelude::*;
use gtk4::{gio, glib};
use voxtype::history::HistoryEntry;

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
}

impl HistoryClient {
    async fn open_meeting(&self, id: &str) -> Result<(), String> {
        let config_path = self.config_path.clone();
        let id = id.to_owned();
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let _thread = std::thread::Builder::new()
            .name("voxtype-history-meeting".into())
            .spawn(move || {
                let result = (|| {
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
                    let result = command
                        .args(["meeting", "view", &id])
                        .stdin(std::process::Stdio::null())
                        .output()
                        .map_err(|error| error.to_string())?;
                    if result.status.success() {
                        Ok(())
                    } else {
                        Err(format!(
                            "Meeting launch failed: {}",
                            String::from_utf8_lossy(&result.stderr)
                        ))
                    }
                })();
                let _ = sender.send(result);
            })
            .map_err(|error| format!("Could not start meeting launch: {error}"))?;
        receiver
            .await
            .map_err(|_| "Meeting launch worker stopped".to_string())?
    }

    async fn history(&self, args: &[&str]) -> Result<serde_json::Value, String> {
        let config_path = self.config_path.clone();
        let args = args.iter().map(|arg| (*arg).to_string()).collect();
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let _thread = std::thread::Builder::new()
            .name("voxtype-history-io".into())
            .spawn(move || {
                let _ = sender.send(Self::history_blocking(config_path, args));
            })
            .map_err(|error| format!("Could not start history request: {error}"))?;
        receiver
            .await
            .map_err(|_| "History request worker stopped".to_string())?
    }

    fn history_blocking(
        config_path: Option<PathBuf>,
        args: Vec<String>,
    ) -> Result<serde_json::Value, String> {
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
        let result = command
            .args(["history", "--json"])
            .args(args)
            .stdin(std::process::Stdio::null())
            .output()
            .map_err(|e| e.to_string())?;
        let value: serde_json::Value = serde_json::from_slice(&result.stdout).map_err(|_| {
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

fn spawn_ui(anchor: &gtk4::Widget, work: impl std::future::Future<Output = ()> + 'static) {
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

fn label(text: &str) -> gtk4::Label {
    let label = gtk4::Label::new(Some(text));
    label.set_xalign(0.0);
    label.set_wrap(true);
    label.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
    label
}

fn update_controls(
    state: &gtk4::Label,
    pause: &gtk4::Button,
    enabled: &gtk4::Button,
    value: &serde_json::Value,
) {
    let collecting = value["enabled"].as_bool().unwrap_or(false);
    let paused = value["paused"].as_bool().unwrap_or(false);
    state.set_text(if !collecting {
        "History off"
    } else if paused {
        "History paused"
    } else {
        "History on"
    });
    pause.set_label(if paused {
        "Resume history"
    } else {
        "Pause history"
    });
    pause.set_visible(collecting);
    pause.set_sensitive(true);
    enabled.set_label(if collecting {
        "Disable history"
    } else {
        "Enable history"
    });
    enabled.set_sensitive(true);
}

fn change_collection(
    toggle_pause: bool,
    state: &gtk4::Label,
    pause: &gtk4::Button,
    enabled: &gtk4::Button,
    notice: &gtk4::Label,
    client: &HistoryClient,
) {
    pause.set_sensitive(false);
    enabled.set_sensitive(false);
    let (state, pause, enabled, notice, client) = (
        state.clone(),
        pause.clone(),
        enabled.clone(),
        notice.clone(),
        client.clone(),
    );
    let anchor = state.clone();
    let generation = client.view.generation.get();
    spawn_ui(anchor.upcast_ref(), async move {
        let result = async {
            let current = client.history(&["status"]).await?;
            let action = if toggle_pause {
                if current["paused"] == true {
                    "resume"
                } else {
                    "pause"
                }
            } else if current["enabled"] == true {
                "disable"
            } else {
                "enable"
            };
            client.history(&[action]).await
        }
        .await;
        if client.view.closed.get() {
            return;
        }
        match result {
            Ok(value) => {
                update_controls(&state, &pause, &enabled, &value);
                if generation == client.view.generation.get() {
                    notice.set_visible(false);
                }
            }
            Err(error) => {
                pause.set_sensitive(true);
                enabled.set_sensitive(true);
                if generation == client.view.generation.get() {
                    notice.set_text(&format!("History change did not complete: {error}"));
                    notice.set_visible(true);
                }
            }
        }
    });
}

fn correction_dialog(
    parent: &gtk4::Window,
    entry: &HistoryEntry,
    list: &gtk4::ListBox,
    ids: &Rc<RefCell<Vec<String>>>,
    client: &HistoryClient,
) {
    let window = gtk4::Window::builder()
        .title("Correct dictation")
        .transient_for(parent)
        .modal(true)
        .default_width(700)
        .default_height(320)
        .build();
    window.set_application(parent.application().as_ref());
    let body = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    body.set_margin_top(18);
    body.set_margin_bottom(18);
    body.set_margin_start(18);
    body.set_margin_end(18);
    body.append(&label(
        "Your correction is stored separately. Raw and output text stay unchanged.",
    ));
    let editor = gtk4::TextView::new();
    editor.set_wrap_mode(gtk4::WrapMode::WordChar);
    editor.buffer().set_text(entry.best_text());
    body.append(
        &gtk4::ScrolledWindow::builder()
            .vexpand(true)
            .child(&editor)
            .build(),
    );
    let notice = label("");
    notice.add_css_class("error");
    notice.set_visible(false);
    body.append(&notice);
    let actions = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    let cancel = gtk4::Button::with_label("Cancel correction");
    let save = gtk4::Button::with_label("Save correction");
    actions.append(&cancel);
    actions.append(&save);
    body.append(&actions);
    {
        let window = window.clone();
        cancel.connect_clicked(move |_| window.close());
    }
    {
        let (window, list, ids, id, client) = (
            window.clone(),
            list.clone(),
            ids.clone(),
            entry.id.clone(),
            client.clone(),
        );
        save.connect_clicked(move |button| {
            button.set_sensitive(false);
            let buffer = editor.buffer();
            let text = buffer
                .text(&buffer.start_iter(), &buffer.end_iter(), true)
                .to_string();
            let (window, list, ids, id, client, notice, button) = (
                window.clone(),
                list.clone(),
                ids.clone(),
                id.clone(),
                client.clone(),
                notice.clone(),
                button.clone(),
            );
            let anchor = window.clone();
            spawn_ui(anchor.upcast_ref(), async move {
                let mut result = client
                    .history(&["correct", &id, "--", &text])
                    .await
                    .map(|_| ());
                if client.view.closed.get() {
                    return;
                }
                if result.is_ok() {
                    // Refresh the live query, not the row's creation-time query.
                    let request = client.view.request();
                    result = reload_list(&list, &ids, &request, &client).await;
                    if !request.is_current() {
                        result = Ok(());
                    }
                }
                if client.view.closed.get() || !window.is_visible() {
                    return;
                }
                match result {
                    Ok(()) => window.close(),
                    Err(error) => {
                        button.set_sensitive(true);
                        notice.set_text(&format!("Correction action did not complete: {error}"));
                        notice.set_visible(true);
                    }
                }
            });
        });
    }
    window.set_child(Some(&body));
    window.present();
}

fn delete_dialog(
    parent: &gtk4::Window,
    id: &str,
    list: &gtk4::ListBox,
    ids: &Rc<RefCell<Vec<String>>>,
    client: &HistoryClient,
) {
    let window = gtk4::Window::builder()
        .title("Delete dictation")
        .transient_for(parent)
        .modal(true)
        .default_width(540)
        .build();
    window.set_application(parent.application().as_ref());
    let body = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    body.set_margin_top(18);
    body.set_margin_bottom(18);
    body.set_margin_start(18);
    body.set_margin_end(18);
    body.append(&label(
        "Delete this dictation from its day file and search index?",
    ));
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
        let (window, list, ids, id, client) = (
            window.clone(),
            list.clone(),
            ids.clone(),
            id.to_string(),
            client.clone(),
        );
        confirm.connect_clicked(move |button| {
            button.set_sensitive(false);
            let (window, list, ids, id, client, notice, button) = (
                window.clone(),
                list.clone(),
                ids.clone(),
                id.clone(),
                client.clone(),
                notice.clone(),
                button.clone(),
            );
            let anchor = window.clone();
            spawn_ui(anchor.upcast_ref(), async move {
                let mut result = client.history(&["delete", &id]).await.map(|_| ());
                if client.view.closed.get() {
                    return;
                }
                if result.is_ok() {
                    let request = client.view.request();
                    result = reload_list(&list, &ids, &request, &client).await;
                    if !request.is_current() {
                        result = Ok(());
                    }
                }
                if client.view.closed.get() || !window.is_visible() {
                    return;
                }
                match result {
                    Ok(()) => window.close(),
                    Err(error) => {
                        button.set_sensitive(true);
                        notice.set_text(&format!("Deletion action did not complete: {error}"));
                        notice.set_visible(true);
                    }
                }
            });
        });
    }
    window.set_child(Some(&body));
    window.present();
}

fn paste_dialog(parent: &gtk4::Window, id: &str, notice: &gtk4::Label, client: &HistoryClient) {
    let window = gtk4::Window::builder()
        .title("Paste dictation")
        .transient_for(parent)
        .modal(true)
        .default_width(540)
        .build();
    window.set_application(parent.application().as_ref());
    let body = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    body.set_margin_top(18);
    body.set_margin_bottom(18);
    body.set_margin_start(18);
    body.set_margin_end(18);
    body.append(&label("History will hide before delivery. Focus the target window during the delay. No new history entry is recorded."));
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
        let (window, parent, id, notice, client) = (
            window.clone(),
            parent.clone(),
            id.to_string(),
            notice.clone(),
            client.clone(),
        );
        confirm.connect_clicked(move |_| {
            // Acquire before unmapping: an unmapped widget need not retain
            // its root, and the idle delay must not outlive the application.
            let hold = parent.application().map(|app| app.hold());
            window.close();
            parent.set_visible(false);
            let (parent, id, notice, client) =
                (parent.clone(), id.clone(), notice.clone(), client.clone());
            // Let GTK unmap the history window before starting the CLI. Keep
            // the application alive so closing the view cannot kill replay.
            glib::timeout_add_local_once(Duration::from_millis(250), move || {
                let anchor = parent.clone();
                spawn_ui(anchor.upcast_ref(), async move {
                    let _hold = hold;
                    let result = client.history(&["paste", &id, "--delay-ms", "500"]).await;
                    if client.view.closed.get() || notice.root().is_none() {
                        return;
                    }
                    if let Err(error) = result {
                        notice.add_css_class("error");
                        notice.set_text(&format!("Replay did not complete: {error}"));
                        notice.set_visible(true);
                        parent.present();
                    }
                });
            });
        });
    }
    window.set_child(Some(&body));
    window.present();
}

async fn reload_list(
    list: &gtk4::ListBox,
    ids: &Rc<RefCell<Vec<String>>>,
    request: &ListRequest,
    client: &HistoryClient,
) -> Result<(), String> {
    let query = request.query.as_str();
    let searching = !query.trim().is_empty();
    let result = if searching {
        client.history(&["search", query]).await
    } else {
        client.history(&["list"]).await
    };
    // Check before propagating failures as well as before touching rows/IDs.
    // A discarded response must not update the caller's current notice either.
    if !request.is_current() {
        return Ok(());
    }
    let value = result?;
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    ids.borrow_mut().clear();
    let values = if searching {
        value.as_array()
    } else {
        value["entries"].as_array()
    }
    .ok_or("History returned an invalid result list")?;
    let mut previous_day = String::new();
    for value in values {
        let row = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
        row.set_margin_top(10);
        row.set_margin_bottom(10);
        row.set_margin_start(10);
        row.set_margin_end(10);
        if value["source"] == "meeting" {
            let id = value["meeting_id"]
                .as_str()
                .ok_or("Meeting search result has no ID")?;
            row.append(&label(&format!(
                "[meeting] {}",
                value["title"].as_str().unwrap_or("Meeting")
            )));
            row.append(&label(value["date"].as_str().unwrap_or("")));
            row.append(&label(value["snippet_plain"].as_str().unwrap_or("")));
            ids.borrow_mut().push(format!("meeting:{id}"));
            let open = gtk4::Button::with_label("Open meeting");
            let status = label("");
            status.set_visible(false);
            row.append(&open);
            row.append(&status);
            let (id, client) = (id.to_owned(), client.clone());
            open.connect_clicked(move |button| {
                let (id, client, status) = (id.clone(), client.clone(), status.clone());
                spawn_ui(button.upcast_ref(), async move {
                    let result = client.open_meeting(&id).await;
                    if client.view.closed.get() {
                        return;
                    }
                    status.set_text(&match result {
                        Ok(()) => "Meeting launch requested.".to_string(),
                        Err(error) => error,
                    });
                    status.set_visible(true);
                });
            });
        } else {
            let raw = if searching { &value["entry"] } else { value };
            let entry: HistoryEntry =
                serde_json::from_value(raw.clone()).map_err(|error| error.to_string())?;
            let day = entry.ts_start.format("%Y-%m-%d").to_string();
            if day != previous_day {
                row.append(&label(&day));
                previous_day = day;
            }
            row.append(&label(&format!(
                "{} {} {}",
                entry.ts_start.format("%H:%M:%S"),
                if entry.delivered { "" } else { "[failed]" },
                if entry.corrected_text.is_some() {
                    "[corrected]"
                } else {
                    ""
                }
            )));
            row.append(&label(&format!(
                "{} | {}",
                entry.app.as_deref().unwrap_or("unknown app"),
                entry.model
            )));
            row.append(&label(entry.best_text()));
            let details = gtk4::Expander::new(Some("Details"));
            let body = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
            body.set_margin_start(12);
            for (heading, text) in [
                ("Raw:", entry.raw_text.as_str()),
                ("Output:", entry.text.as_str()),
                (
                    "Correction:",
                    entry.corrected_text.as_deref().unwrap_or("(none)"),
                ),
            ] {
                body.append(&label(heading));
                let value = label(text);
                value.set_selectable(true);
                body.append(&value);
            }
            body.append(&label("Automatic edits:"));
            if entry.edits.is_empty() {
                body.append(&label("(none)"));
            } else {
                for edit in &entry.edits {
                    body.append(&label(&format!("- {}\n+ {}", edit.from, edit.to)));
                }
            }
            body.append(&label(&format!("Engine: {} | backend: {} | outcome: {}\nAudio: {} ms | transcription: {} | model load: {}", entry.engine, entry.backend.as_deref().unwrap_or("unknown"), entry.outcome.as_str(), entry.audio_ms, entry.transcribe_ms.map(|value| format!("{value} ms")).unwrap_or_else(|| "unknown".into()), entry.model_load_ms.map(|value| format!("{value} ms")).unwrap_or_else(|| "unknown".into()))));
            if let Some(error) = &entry.error {
                body.append(&label(error));
            }
            details.set_child(Some(&body));
            row.append(&details);
            let correct = gtk4::Button::with_label("Correct");
            {
                let (entry, list, ids, client) =
                    (entry.clone(), list.clone(), ids.clone(), client.clone());
                correct.connect_clicked(move |button| {
                    if let Some(parent) = button
                        .root()
                        .and_then(|root| root.downcast::<gtk4::Window>().ok())
                    {
                        correction_dialog(&parent, &entry, &list, &ids, &client);
                    }
                });
            }
            let actions = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
            actions.append(&correct);
            let delete = gtk4::Button::with_label("Delete");
            {
                let (id, list, ids, client) =
                    (entry.id.clone(), list.clone(), ids.clone(), client.clone());
                delete.connect_clicked(move |button| {
                    if let Some(parent) = button
                        .root()
                        .and_then(|root| root.downcast::<gtk4::Window>().ok())
                    {
                        delete_dialog(&parent, &id, &list, &ids, &client);
                    }
                });
            }
            actions.append(&delete);
            let copy = gtk4::Button::with_label("Copy");
            let notice = label("");
            notice.set_visible(false);
            {
                let (id, notice, client) = (entry.id.clone(), notice.clone(), client.clone());
                copy.connect_clicked(move |button| {
                    button.set_sensitive(false);
                    let (id, notice, client, button) =
                        (id.clone(), notice.clone(), client.clone(), button.clone());
                    let anchor = button.clone();
                    spawn_ui(anchor.upcast_ref(), async move {
                        let result = client.history(&["copy", &id]).await;
                        if client.view.closed.get() || button.root().is_none() {
                            return;
                        }
                        match result {
                            Ok(_) => {
                                notice.remove_css_class("error");
                                notice.set_text("Copied transcript.");
                            }
                            Err(error) => {
                                notice.add_css_class("error");
                                notice.set_text(&format!("Copy did not complete: {error}"));
                            }
                        }
                        button.set_sensitive(true);
                        notice.set_visible(true);
                    });
                });
            }
            actions.append(&copy);
            let paste = gtk4::Button::with_label("Paste again");
            {
                let (id, notice, client) = (entry.id.clone(), notice.clone(), client.clone());
                paste.connect_clicked(move |button| {
                    if let Some(parent) = button
                        .root()
                        .and_then(|root| root.downcast::<gtk4::Window>().ok())
                    {
                        paste_dialog(&parent, &id, &notice, &client);
                    }
                });
            }
            actions.append(&paste);
            row.append(&actions);
            row.append(&notice);
            ids.borrow_mut().push(entry.id);
        }
        list.append(&row);
    }
    if ids.borrow().is_empty() {
        list.append(&label(if searching {
            "No matching dictations or meetings"
        } else {
            "No dictations stored"
        }));
    }
    list.select_row(list.row_at_index(0).as_ref());
    Ok(())
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
    let existing = Rc::new(RefCell::new(std::collections::HashMap::<
        Option<PathBuf>,
        gtk4::ApplicationWindow,
    >::new()));
    app.connect_command_line(move |app, command_line| {
        let arguments = command_line.arguments();
        let config_path = arguments.windows(2).find(|pair| pair[0] == "--config").map(|pair| {
            let path = PathBuf::from(&pair[1]);
            if path.is_absolute() { Ok(path) } else {
                // GApplication forwards the caller's cwd with its arguments.
                // Direct helper invocations need the same isolation as the
                // canonical CLI, without changing the primary's global cwd.
                command_line.cwd().map(|cwd| cwd.join(path))
                    .ok_or("Cannot resolve relative --config: invocation working directory is unavailable")
            }
        }).transpose();
        let config_path = match config_path {
            Ok(path) => path,
            Err(error) => { eprintln!("{error}"); return glib::ExitCode::FAILURE; }
        };
        if let Some(window) = existing.borrow().get(&config_path) {
            window.present();
            return glib::ExitCode::SUCCESS;
        }
        // Each configuration keeps its own immutable command context. A later
        // forwarded launch must never retarget another window's pending action.
        let client = HistoryClient { config_path: config_path.clone(), view: Rc::default() };
        let window = gtk4::ApplicationWindow::builder().application(app)
            .title("Voxtype History").default_width(1000).default_height(760).build();
        {
            let (existing, key, view) = (existing.clone(), config_path.clone(), client.view.clone());
            window.connect_close_request(move |_| {
                view.closed.set(true);
                view.invalidate(String::new());
                existing.borrow_mut().remove(&key);
                glib::Propagation::Proceed
            });
        }
        window.set_widget_name("history");
        let content = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
        content.set_margin_top(18);
        content.set_margin_bottom(18);
        content.set_margin_start(18);
        content.set_margin_end(18);
        let state = label("Loading history");
        state.add_css_class("title-1");
        content.append(&state);
        let controls = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        let pause = gtk4::Button::with_label("Pause history");
        let enabled = gtk4::Button::with_label("Enable history");
        pause.set_sensitive(false);
        enabled.set_sensitive(false);
        controls.append(&pause);
        controls.append(&enabled);
        content.append(&controls);
        let notice = label("");
        notice.add_css_class("error");
        notice.set_visible(false);
        content.append(&notice);
        {
            let (state, pause_control, enabled, notice, client) = (state.clone(), pause.clone(), enabled.clone(), notice.clone(), client.clone());
            pause.connect_clicked(move |_| change_collection(true, &state, &pause_control, &enabled, &notice, &client));
        }
        {
            let (state, pause, enabled_control, notice, client) = (state.clone(), pause.clone(), enabled.clone(), notice.clone(), client.clone());
            enabled.connect_clicked(move |_| change_collection(false, &state, &pause, &enabled_control, &notice, &client));
        }
        let search = gtk4::SearchEntry::new();
        search.set_placeholder_text(Some("Search dictations and meetings"));
        content.append(&search);
        let list = gtk4::ListBox::new();
        list.set_selection_mode(gtk4::SelectionMode::Single);
        let scroll = gtk4::ScrolledWindow::builder().vexpand(true).child(&list).build();
        content.append(&scroll);
        let ids = Rc::new(RefCell::new(Vec::new()));
        {
            let view = client.view.clone();
            // Invalidate immediately, not only after SearchEntry's debounce.
            search.connect_changed(move |search| view.invalidate(search.text().to_string()));
        }
        {
            let (list, ids, notice, client) = (list.clone(), ids.clone(), notice.clone(), client.clone());
            search.connect_search_changed(move |search| {
                let request = client.view.request();
                let (list, ids, notice, client) = (list.clone(), ids.clone(), notice.clone(), client.clone());
                spawn_ui(search.upcast_ref(), async move {
                    let result = reload_list(&list, &ids, &request, &client).await;
                    if !request.is_current() { return; }
                    match result {
                        Ok(()) => notice.set_visible(false),
                        Err(error) => { notice.set_text(&format!("History search did not complete: {error}")); notice.set_visible(true); }
                    }
                });
            });
        }
        list.select_row(list.row_at_index(0).as_ref());
        window.set_child(Some(&content));
        {
            let (state, pause, enabled, notice, list, ids, client) = (state.clone(), pause.clone(), enabled.clone(), notice.clone(), list.clone(), ids.clone(), client.clone());
            // Reserve before spawning or awaiting initial status, so a newer
            // search cannot be superseded by a late initial list request.
            let request = client.view.request();
            spawn_ui(window.upcast_ref(), async move {
                let status = client.history(&["status"]).await;
                if client.view.closed.get() { return; }
                match status {
                    Ok(value) => {
                        update_controls(&state, &pause, &enabled, &value);
                        if !request.is_current() { return; }
                        let result = reload_list(&list, &ids, &request, &client).await;
                        if !request.is_current() { return; }
                        if let Err(error) = result {
                            notice.set_text(&error);
                            notice.set_visible(true);
                        }
                    }
                    Err(error) => {
                        state.set_text("History state unavailable");
                        if request.is_current() {
                            notice.set_text(&error);
                            notice.set_visible(true);
                        }
                    }
                }
            });
        }
        if let Some(seconds) = std::env::var(super::SMOKE_ENV).ok().and_then(|s| s.parse::<u64>().ok()) {
            let (window, state, list, app) = (window.clone(), state.clone(), list.clone(), app.clone());
            glib::timeout_add_local_once(Duration::from_secs(seconds), move || {
                let selected_id = list.selected_row().and_then(|row| ids.borrow().get(row.index() as usize).cloned());
                println!("{}",serde_json::json!({"screen":window.widget_name().to_string(),"state":state.text().to_string(),"entries":ids.borrow().len(),"selected_id":selected_id}));
                if let Some(path) = std::env::var_os(super::SHOT_ENV) {
                    super::save_png(&window, std::path::Path::new(&path));
                }
                app.quit();
            });
        }
        window.present();
        existing.borrow_mut().insert(config_path, window);
        glib::ExitCode::SUCCESS
    });
    app.run()
}
