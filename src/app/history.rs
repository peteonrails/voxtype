//! History frontends share the CLI's JSON contract and authoritative day files.
use std::io::{IsTerminal, Write};
use std::path::PathBuf;

use anyhow::{bail, Context};
use chrono::NaiveDate;
use serde_json::{json, Value};
use voxtype::history::{index, HistoryEntry, HistoryIndex, HistoryQuery, HistoryStore};
use voxtype::{config, config_set, output, HistoryAction};

pub(crate) async fn run(
    config: &config::Config,
    config_path: Option<PathBuf>,
    action: HistoryAction,
    json_output: bool,
) -> anyhow::Result<()> {
    if matches!(action, HistoryAction::View { tui: false }) && !json_output {
        // Resolve while still in the invoking CLI, before any GUI IPC can
        // transfer this request to a primary process in another directory.
        // Do not canonicalize: a missing explicit path must retain its normal
        // default-read/write-error behavior rather than fail here.
        let config_path = config_path
            .as_ref()
            .map(std::path::absolute)
            .transpose()
            .context("Could not resolve the history configuration path")?;
        if open_quickshell(config_path.as_deref()).await? {
            println!("Opened the history window.");
            return Ok(());
        }
        use voxtype::meeting::view::frontend::{resolve, ViewFrontend};
        let frontend = resolve(ViewFrontend::Gtk4, &voxtype::osd::frontend::SystemProbe);
        if let Some(path) = frontend
            .path
            .filter(|_| frontend.active == ViewFrontend::Gtk4)
        {
            let mut command = std::process::Command::new(path);
            command.arg("--history");
            if let Some(path) = &config_path {
                command.arg("--config").arg(path);
            }
            command
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .context("Could not open the GTK history window")?;
            println!("Opened the history window.");
            return Ok(());
        }
    }
    if matches!(action, HistoryAction::View { .. }) && !json_output {
        return voxtype::tui::history_view::run(config_path);
    }
    let result = execute(config, config_path, action).await;
    match result {
        Ok(value) => {
            if json_output {
                println!("{}", serde_json::to_string(&value)?);
            } else {
                print_human(&value);
            }
            Ok(())
        }
        Err(error) => {
            if json_output {
                println!("{}", json!({"error": error.to_string()}));
            }
            Err(error)
        }
    }
}

async fn ipc_ok(program: &std::path::Path, args: &[&std::ffi::OsStr]) -> bool {
    let mut command = tokio::process::Command::new(program);
    command
        .args(args)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    match tokio::time::timeout(std::time::Duration::from_secs(2), command.output()).await {
        Ok(Ok(output)) => {
            output.status.success() && String::from_utf8_lossy(&output.stdout).trim() == "ok"
        }
        _ => false,
    }
}

async fn open_quickshell(config_path: Option<&std::path::Path>) -> anyhow::Result<bool> {
    use voxtype::osd::frontend::{OsdProbe, SystemProbe};
    if !SystemProbe.wayland_session() {
        return Ok(false);
    }
    let omarchy = which::which("omarchy-shell").ok().or_else(|| {
        let path = PathBuf::from("/usr/share/omarchy/bin/omarchy-shell");
        path.is_file().then_some(path)
    });
    if let Some(program) = omarchy {
        let mut args = vec![std::ffi::OsStr::new("io.voxtype.settings")];
        if let Some(path) = config_path {
            args.extend([std::ffi::OsStr::new("historyConfig"), path.as_os_str()]);
        } else {
            args.push(std::ffi::OsStr::new("history"));
        }
        if ipc_ok(&program, &args).await {
            return Ok(true);
        }
    }
    let directory = std::env::var_os("VOXTYPE_OSD_QML_PATH")
        .map(PathBuf::from)
        .map(|path| {
            if path.is_file() {
                path.parent().unwrap_or(&path).to_path_buf()
            } else {
                path
            }
        })
        .filter(|path| path.join("history.qml").is_file())
        .or_else(|| {
            let path = voxtype::setup::quickshell::default_target_dir();
            path.join("history.qml").is_file().then_some(path)
        })
        .or_else(|| voxtype::setup::quickshell::resolve_source_dir(None));
    let Some(directory) = directory else {
        return Ok(false);
    };
    let entry = directory.join("history.qml");
    if !entry.is_file() {
        return Ok(false);
    }
    let Some(program) =
        voxtype::osd::frontend::find_quickshell_program(|name| which::which(name).ok())
    else {
        return Ok(false);
    };
    let mut args = vec![
        std::ffi::OsStr::new("ipc"),
        std::ffi::OsStr::new("-p"),
        entry.as_os_str(),
        std::ffi::OsStr::new("call"),
        std::ffi::OsStr::new("history"),
    ];
    if let Some(path) = config_path {
        args.extend([std::ffi::OsStr::new("openConfig"), path.as_os_str()]);
    } else {
        args.push(std::ffi::OsStr::new("open"));
    }
    if ipc_ok(&program, &args).await {
        return Ok(true);
    }
    let mut command = std::process::Command::new(&program);
    command
        .arg("-d")
        .arg("-p")
        .arg(&entry)
        .env("VOXTYPE_HISTORY_BIN", std::env::current_exe()?)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    if let Some(path) = config_path {
        command.env("VOXTYPE_HISTORY_CONFIG", path);
    }
    if command.spawn().is_err() {
        return Ok(false);
    }
    // Bound startup as a whole. Thirty individually bounded IPC probes can
    // otherwise delay a working GTK fallback for more than a minute.
    let startup = async {
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            if ipc_ok(&program, &args).await {
                return true;
            }
        }
    };
    Ok(
        tokio::time::timeout(std::time::Duration::from_secs(3), startup)
            .await
            .unwrap_or(false),
    )
}

fn date(value: Option<String>) -> anyhow::Result<Option<NaiveDate>> {
    value
        .map(|s| {
            let day = NaiveDate::parse_from_str(&s, "%Y-%m-%d")
                .context("Use a date in YYYY-MM-DD format")?;
            if day.to_string() != s {
                bail!("Use a date in YYYY-MM-DD format");
            }
            Ok(day)
        })
        .transpose()
}

fn find(store: &HistoryStore, id: &str) -> anyhow::Result<HistoryEntry> {
    let entry = if id == "last" {
        store.recent(1)?.into_iter().next()
    } else {
        store.find(id)?
    };
    entry.with_context(|| format!("No dictation found for '{id}'"))
}

fn state(config: &config::Config, store: &HistoryStore) -> anyhow::Result<Value> {
    Ok(json!({
        "enabled": config.history.enabled,
        "paused": config.history.paused,
        "recording": config.history.recording(),
        "retention_days": config.history.retention_days,
        "count": store.all()?.len(),
    }))
}

async fn execute(
    config: &config::Config,
    config_path: Option<PathBuf>,
    action: HistoryAction,
) -> anyhow::Result<Value> {
    let store = HistoryStore::at(HistoryStore::default_dir());
    let db = index::db_path(config);
    match action {
        HistoryAction::View { .. } => {
            let mut result = state(config, &store)?;
            result["entries"] = serde_json::to_value(store.recent(200)?)?;
            Ok(result)
        }
        HistoryAction::List { limit, failed } => {
            let entries = if failed {
                let mut entries = store.all()?;
                entries.retain(|e| !e.delivered);
                entries.sort_by_key(|e| std::cmp::Reverse(e.ts_start));
                entries.truncate(limit);
                entries
            } else {
                store.recent(limit)?
            };
            let mut result = state(config, &store)?;
            result["entries"] = serde_json::to_value(entries)?;
            Ok(result)
        }
        HistoryAction::Status => state(config, &store),
        HistoryAction::Show { id } => Ok(serde_json::to_value(find(&store, &id)?)?),
        HistoryAction::Search {
            words,
            limit,
            since,
        } => {
            let query = HistoryQuery {
                text: words.join(" "),
                limit,
                since: date(since)?,
            };
            let index = HistoryIndex::open(&db)?;
            index.sync(&store)?;
            let meetings =
                voxtype::meeting::MeetingStorage::open(voxtype::meeting::StorageConfig {
                    storage_path: db
                        .parent()
                        .context("Invalid shared history index path")?
                        .to_path_buf(),
                    ..Default::default()
                })?;
            meetings.backfill_search_index()?;
            Ok(serde_json::to_value(index::search_everything(
                &index,
                &store,
                Some(&meetings),
                &query,
            )?)?)
        }
        HistoryAction::Correct { id, text } => {
            let entry = find(&store, &id)?;
            // What the user was looking at when they made the change.
            let before = entry.best_text().to_string();
            let suggestion =
                voxtype::history::suggest_rule(&before, &text, &config.text.replacements);
            let corrected = store
                .update(&entry.id, |e| e.corrected_text = Some(text))?
                .context("Dictation was deleted before the correction could be saved")?;
            HistoryIndex::open(&db)?.sync(&store)?;
            let mut result = serde_json::to_value(corrected)?;
            if let Some(rule) = suggestion {
                result["rule_suggestion"] = serde_json::to_value(rule)?;
            }
            Ok(result)
        }
        HistoryAction::Rule { from, to } => {
            let from = from.trim().to_lowercase();
            let to = to.trim().to_string();
            if from.is_empty() || to.is_empty() {
                bail!("A replacement needs both the dictated text and what to type in its place");
            }
            if from.contains('.') {
                bail!(
                    "The dictated text can't contain a period here. Add \"{from}\" = \"{to}\" \
                     under [text.replacements] in the config file instead."
                );
            }
            let replaced = config
                .text
                .replacements
                .iter()
                .find(|(k, _)| k.to_lowercase() == from)
                .map(|(_, v)| v.clone());
            let path = super::config_set::resolve_config_path_for_write(config_path)?;
            let outcome = config_set::set_key(path, &format!("text.replacements.{from}"), &to)?;
            Ok(json!({
                "from": from,
                "to": to,
                "replaced": replaced,
                "restart_required": outcome.restart_required,
            }))
        }
        HistoryAction::Delete { id } => {
            let entry = find(&store, &id)?;
            store
                .delete(&entry.id)?
                .context("Dictation was already deleted")?;
            HistoryIndex::open(&db)?.sync(&store)?;
            Ok(json!({"deleted": entry.id}))
        }
        HistoryAction::Clear { before, yes } => {
            let before = date(before)?;
            if !yes {
                if !std::io::stdin().is_terminal() {
                    bail!("Use --yes to confirm clearing history when not on a terminal");
                }
                eprint!(
                    "Delete {} history? Type yes: ",
                    before
                        .map(|d| format!("days before {d}"))
                        .unwrap_or_else(|| "all".into())
                );
                std::io::stderr().flush()?;
                let mut answer = String::new();
                std::io::stdin().read_line(&mut answer)?;
                if answer.trim() != "yes" {
                    bail!("History was not cleared");
                }
            }
            let removed = store.clear(before)?;
            HistoryIndex::open(&db)?.sync(&store)?;
            Ok(json!({"days": removed.len()}))
        }
        HistoryAction::Reindex => {
            let index = HistoryIndex::open(&db)?;
            let days = index.rebuild(&store)?;
            Ok(json!({"days":days,"indexed":index.count()?}))
        }
        HistoryAction::Pause
        | HistoryAction::Resume
        | HistoryAction::Enable
        | HistoryAction::Disable => {
            let (key, value) = match action {
                HistoryAction::Pause => ("history.paused", "true"),
                HistoryAction::Resume => ("history.paused", "false"),
                HistoryAction::Enable => ("history.enabled", "true"),
                HistoryAction::Disable => ("history.enabled", "false"),
                _ => unreachable!(),
            };
            let path = super::config_set::resolve_config_path_for_write(config_path)?;
            config_set::set_key(path.clone(), key, value)?;
            let saved = config::load_config(Some(&path))?;
            state(&saved, &store)
        }
        HistoryAction::Retention { days } => {
            let path = super::config_set::resolve_config_path_for_write(config_path)?;
            config_set::set_key(path.clone(), "history.retention_days", &days.to_string())?;
            let saved = config::load_config(Some(&path))?;
            state(&saved, &store)
        }
        HistoryAction::Copy { id } => {
            let entry = find(&store, id.as_deref().unwrap_or("last"))?;
            let clipboard = config::OutputConfig {
                mode: config::OutputMode::Clipboard,
                append_text: None,
                ..Default::default()
            };
            let mut errors = Vec::new();
            for driver in output::create_output_chain(&clipboard) {
                if driver.is_available().await {
                    match driver.output(entry.best_text()).await {
                        Ok(()) => return Ok(json!({"copied":entry.id,"driver":driver.name()})),
                        Err(e) => errors.push(e.to_string()),
                    }
                }
            }
            bail!("No clipboard driver succeeded. Install wl-copy (Wayland), xclip (X11), or pbcopy (macOS). {}", errors.join("; "))
        }
        HistoryAction::Paste { id, delay_ms } => {
            let entry = find(&store, &id)?;
            tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
            if config.output.mode == config::OutputMode::File {
                if let Some(path) = &config.output.file_path {
                    let fallback = if config.output.fallback_to_clipboard {
                        let mut clipboard = config.output.clone();
                        clipboard.mode = config::OutputMode::Clipboard;
                        output::create_output_chain(&clipboard)
                    } else {
                        Vec::new()
                    };
                    use voxtype::daemon::{deliver_to_file_observed, FileDelivery};
                    return match deliver_to_file_observed(
                        path,
                        entry.best_text(),
                        &config.output,
                        &fallback,
                        None,
                    )
                    .await
                    {
                        FileDelivery::Written => Ok(
                            json!({"pasted":entry.id,"driver":"file","mode":"file","file":path,"fell_back":false}),
                        ),
                        FileDelivery::FellBack(error) => Ok(
                            json!({"pasted":entry.id,"driver":"clipboard","mode":"file","file":path,"fell_back":true,"file_error":error}),
                        ),
                        FileDelivery::Lost(error) => bail!(
                            "Replay could not write {} and no fallback delivered the text: {error}",
                            path.display()
                        ),
                    };
                }
            }
            let chain = output::create_output_chain(&config.output);
            let delivered = output::deliver_with_fallback(
                &chain,
                entry.best_text(),
                output::OutputOptions {
                    history_context: None,
                    pre_output_command: config.output.pre_output_command.as_deref(),
                    post_output_command: config.output.post_output_command.as_deref(),
                    wait_for_modifier_release: config.output.wait_for_modifier_release,
                    modifier_release_timeout: std::time::Duration::from_millis(
                        config.output.modifier_release_timeout_ms,
                    ),
                },
            )
            .await?;
            Ok(json!({"pasted":entry.id,"driver":delivered.driver,"fell_back":delivered.fell_back}))
        }
    }
}

fn print_human(value: &Value) {
    if let Some(entries) = value.get("entries").and_then(Value::as_array) {
        println!(
            "History: {}{}",
            if value["enabled"] == true {
                "on"
            } else {
                "off"
            },
            if value["paused"] == true {
                " (paused)"
            } else {
                ""
            }
        );
        for entry in entries {
            println!(
                "{}  {}  {}{}  {}",
                entry["id"].as_str().unwrap_or(""),
                entry["app"].as_str().unwrap_or("unknown app"),
                entry["outcome"].as_str().unwrap_or(""),
                if entry.get("corrected_text").is_some() {
                    " [corrected]"
                } else {
                    ""
                },
                entry["text"].as_str().unwrap_or("")
            );
        }
        if entries.is_empty() {
            println!("No dictations stored.");
        }
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(value).unwrap_or_default()
        );
    }
}
