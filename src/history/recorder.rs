//! The daemon's side of history: build an entry while a dictation runs and
//! write it once the dictation is over.
//!
//! Writing never delays output. The entry is committed when the daemon goes
//! back to idle, which is after delivery. Destination context is captured
//! around delivery, before post-output hooks; file/index writes run separately.
//!
//! Settings are re-read from the config file whenever it changes, so
//! "Enable History" in the tray and `voxtype history pause` take effect on
//! the next dictation without a restart. Daemon flags still win afterwards.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime};

use chrono::{Local, NaiveDate};

use super::entry::{EndedBy, HistoryEntry, Outcome};
use super::focus::{self, FocusedWindow};
use super::index::{self, HistoryIndex};
use super::store::HistoryStore;
use crate::config::{Config, HistoryConfig, HistoryOverrides};

/// A dictation in progress.
pub struct Pending {
    pub entry: HistoryEntry,
    /// Whether `entry.outcome` has been decided. The first outcome wins,
    /// as with the OSD's: later paths only add detail.
    outcome_set: bool,
    started: Instant,
    transcribe_started: Option<Instant>,
    /// The window focused when the recording started.
    start_window: Option<tokio::task::JoinHandle<Option<FocusedWindow>>>,
    delivery_context: DeliveryContext,
    privacy_generation: Option<u64>,
}

/// Focus observed at delivery boundaries, not later background-writer focus.
/// An excluded attempt cannot be overwritten by a fallback or restored focus.
#[derive(Clone, Default)]
pub struct DeliveryContext(Arc<Mutex<Vec<Option<FocusedWindow>>>>);

impl DeliveryContext {
    pub(crate) async fn capture(&self) {
        let window = focus::focused_window().await;
        let mut windows = self.0.lock().unwrap_or_else(|error| error.into_inner());
        if windows.last() != Some(&window) {
            windows.push(window);
        }
    }

    fn windows(&self) -> Vec<Option<FocusedWindow>> {
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }
}

impl Pending {
    pub fn delivery_context(&self) -> DeliveryContext {
        self.delivery_context.clone()
    }

    pub fn outcome(&mut self, outcome: Outcome, error: Option<String>) {
        if self.outcome_set {
            if error.is_some() && self.entry.error.is_none() && self.entry.outcome == outcome {
                self.entry.error = error;
            }
            return;
        }
        self.outcome_set = true;
        self.entry.set_outcome(outcome, error);
        if outcome == Outcome::Cancelled {
            self.entry.ended_by = EndedBy::Cancelled;
        }
    }

    /// Transcription is starting on `audio_samples` 16 kHz samples.
    pub fn transcribing(&mut self, audio_samples: usize) {
        self.entry.audio_ms = (audio_samples as u64) / 16;
        self.transcribe_started.get_or_insert_with(Instant::now);
    }

    /// Transcription finished.
    pub fn transcribed(&mut self) {
        if let Some(t) = self.transcribe_started {
            self.entry.transcribe_ms = Some(t.elapsed().as_millis() as u64);
        }
    }

    /// The engine's text and what was output from it.
    pub fn texts(&mut self, raw: &str, text: &str) {
        self.entry.raw_text = raw.to_string();
        self.entry.text = text.to_string();
        self.entry.edits = super::entry::edits_between(raw, text);
    }

    /// The text an output method actually received, when delivery changed
    /// it on the way (curly quotes become straight ones for the keyboard
    /// tools). `text` is what the user got, so it is what Copy and Paste
    /// again must reproduce; the engine's wording stays in `raw_text`.
    pub fn delivered_text(&mut self, delivered: &str) {
        if self.entry.text != delivered {
            self.entry.text = delivered.to_string();
            self.entry.edits = super::entry::edits_between(&self.entry.raw_text, delivered);
        }
    }
}

/// What the daemon keeps between dictations.
#[derive(Clone)]
pub struct Recorder {
    /// `--config`, or `None` for the usual lookup.
    config_path: Option<PathBuf>,
    settings: HistoryConfig,
    /// The config file as last read: modification time and size.
    seen: Option<(SystemTime, u64)>,
    store_dir: PathBuf,
    index_db: PathBuf,
    last_maintenance: Option<NaiveDate>,
    #[cfg(test)]
    pub(crate) test_finished: Option<std::sync::mpsc::Sender<()>>,
}

impl Recorder {
    pub fn new(config: &Config, config_path: Option<PathBuf>) -> Self {
        let mut recorder = Self {
            config_path,
            settings: config.history.clone(),
            seen: None,
            store_dir: HistoryStore::default_dir(),
            index_db: index::db_path(config),
            last_maintenance: None,
            #[cfg(test)]
            test_finished: None,
        };
        recorder.seen = recorder.file_stamp();
        recorder
    }

    /// A recorder writing to explicit places, for tests.
    pub fn with_paths(settings: HistoryConfig, store_dir: PathBuf, index_db: PathBuf) -> Self {
        Self {
            config_path: None,
            settings,
            seen: None,
            store_dir,
            index_db,
            last_maintenance: None,
            #[cfg(test)]
            test_finished: None,
        }
    }

    fn file_path(&self) -> Option<PathBuf> {
        // Unit tests build daemons without a config path; they must never
        // pick up the developer's own config and start writing history.
        if cfg!(test) && self.config_path.is_none() {
            return None;
        }
        self.config_path
            .clone()
            .or_else(Config::resolve_existing_path)
    }

    fn file_stamp(&self) -> Option<(SystemTime, u64)> {
        let meta = std::fs::metadata(self.file_path()?).ok()?;
        Some((meta.modified().ok()?, meta.len()))
    }

    /// Re-read `[history]` if the config file changed since it was last
    /// read. A file that no longer parses keeps the settings in force.
    pub fn refresh(&mut self) -> &HistoryConfig {
        let stamp = self.file_stamp();
        if stamp.is_some() && stamp != self.seen {
            self.seen = stamp;
            match crate::config::load_config(self.config_path.as_deref()) {
                Ok(config) => {
                    let mut settings = config.history;
                    HistoryOverrides::cli().apply(&mut settings);
                    if settings != self.settings {
                        tracing::info!("History settings changed: {}", describe(&settings));
                    }
                    self.settings = settings;
                }
                Err(e) => tracing::warn!("Could not re-read [history]: {}", e),
            }
        }
        &self.settings
    }

    pub fn settings(&self) -> &HistoryConfig {
        &self.settings
    }

    pub fn store_dir(&self) -> &PathBuf {
        &self.store_dir
    }

    /// Start a dictation, or `None` when history is off or paused.
    pub fn begin(&mut self) -> Option<Pending> {
        if !self.refresh().recording() {
            return None;
        }
        let path = self.file_path();
        let _privacy = path
            .as_deref()
            .map(super::store::privacy_lock)
            .transpose()
            .map_err(|error| {
                tracing::warn!(
                    "History privacy lock unavailable; not recording this dictation: {}",
                    error
                )
            })
            .ok()?;
        self.seen = None;
        if !self.refresh().recording() {
            return None;
        }
        let privacy_generation = path
            .as_deref()
            .map(super::store::privacy_generation)
            .transpose()
            .map_err(|error| {
                tracing::warn!(
                    "History privacy state unavailable; not recording this dictation: {}",
                    error
                )
            })
            .ok()?;
        let start_window = tokio::runtime::Handle::try_current()
            .ok()
            .map(|rt| rt.spawn(focus::focused_window()));
        Some(Pending {
            entry: HistoryEntry::new(Local::now()),
            outcome_set: false,
            started: Instant::now(),
            transcribe_started: None,
            start_window,
            delivery_context: DeliveryContext::default(),
            privacy_generation,
        })
    }

    /// Finish a dictation and write it in the background. Returns the task
    /// so tests can wait for it.
    pub fn commit(&mut self, mut pending: Pending) -> Option<tokio::task::JoinHandle<()>> {
        // Paused (or turned off) since the dictation started: write nothing.
        if !self.refresh().recording() {
            tracing::debug!("History is off or paused; not recording this dictation");
            return None;
        }
        if !pending.outcome_set {
            pending.outcome(
                Outcome::TranscriptionFailed,
                Some("the dictation ended without a result".into()),
            );
        }
        pending.entry.ts_end = (pending.entry.ts_start
            + chrono::Duration::from_std(pending.started.elapsed()).ok()?)
        .fixed_offset();
        let mut write_guard = self.clone();
        let store_dir = self.store_dir.clone();
        let index_db = self.index_db.clone();
        let today = Local::now().date_naive();
        let maintain = self.last_maintenance != Some(today);
        self.last_maintenance = Some(today);
        let rt = tokio::runtime::Handle::try_current().ok()?;
        #[cfg(test)]
        let finished = self.test_finished.clone();
        Some(rt.spawn(async move {
            let start = match pending.start_window.take() {
                Some(task) => task.await.ok().flatten(),
                None => None,
            };
            let destinations = pending.delivery_context.windows();
            let privacy_generation = pending.privacy_generation;
            let mut entry = pending.entry;
            let result = tokio::task::spawn_blocking(move || {
                let mut retention = 0;
                let written = write(&store_dir, &index_db, &mut entry, |entry| {
                    let path = write_guard.file_path();
                    let privacy = path
                        .as_deref()
                        .map(super::store::privacy_lock)
                        .transpose()?;
                    if path
                        .as_deref()
                        .map(super::store::privacy_generation)
                        .transpose()?
                        != privacy_generation
                    {
                        return Ok(None);
                    }
                    // This read and append are serialized with config saves,
                    // after any wait for storage. Do not trust a cached stamp.
                    write_guard.seen = None;
                    let settings = write_guard.refresh();
                    if !settings.recording()
                        || !apply_delivery_context(settings, entry, start, destinations)
                    {
                        return Ok(None);
                    }
                    retention = settings.retention_days;
                    Ok(Some(privacy))
                })?;
                if !written {
                    return Ok(None);
                }
                if maintain {
                    maintenance(&store_dir, &index_db, retention, today);
                }
                Ok::<_, String>(Some(entry))
            })
            .await;
            match result {
                Ok(Ok(Some(entry))) => tracing::debug!(
                    "History: recorded {} ({})",
                    entry.id,
                    entry.outcome.as_str()
                ),
                Ok(Ok(None)) => {
                    tracing::debug!("History write discarded by current privacy settings")
                }
                Ok(Err(e)) => tracing::warn!("Could not record dictation history: {}", e),
                Err(e) => tracing::warn!("History write task failed: {}", e),
            }
            #[cfg(test)]
            if let Some(finished) = finished {
                let _ = finished.send(());
            }
        }))
    }
}

/// Fill in the app and window title from the windows focused when the
/// dictation started and ended, as the settings allow. Returns `false` when
/// either window belongs to an excluded app: the dictation must not be
/// written at all.
pub fn apply_context(
    settings: &HistoryConfig,
    entry: &mut HistoryEntry,
    start: Option<FocusedWindow>,
    end: Option<FocusedWindow>,
) -> bool {
    for window in [&start, &end].into_iter().flatten() {
        if let Some(rule) = settings.excluded_by(&window.app) {
            tracing::debug!(
                "Not recording a dictation into {} (history.exclude_apps: {})",
                window.app,
                rule
            );
            return false;
        }
    }
    // Where the text went: the window focused once output was done.
    if let Some(window) = end.or(start) {
        if settings.record_app && !window.app.is_empty() {
            entry.app = Some(window.app);
        }
        if settings.record_window_title && !window.title.is_empty() {
            entry.window_title = Some(window.title);
        }
    }
    true
}

fn apply_delivery_context(
    settings: &HistoryConfig,
    entry: &mut HistoryEntry,
    start: Option<FocusedWindow>,
    destinations: Vec<Option<FocusedWindow>>,
) -> bool {
    if start
        .iter()
        .chain(destinations.iter().flatten())
        .any(|window| settings.excluded_by(&window.app).is_some())
    {
        return false;
    }
    let destination = match destinations.last() {
        Some(window) => window.clone(),
        None => start,
    };
    // A failed lookup at delivery is unknown, not the recording-start app.
    entry.app = None;
    entry.window_title = None;
    apply_context(settings, entry, None, destination)
}

/// The running binary's variant (`avx2`, `onnx-migraphx`, ...), from its
/// file name, and the GPU backend that variant was built for (`cpu` when
/// none). This is what the binary can use, not proof a GPU ran the model.
pub fn binary_variant_and_backend() -> (Option<String>, Option<String>) {
    static CACHED: std::sync::OnceLock<(Option<String>, Option<String>)> =
        std::sync::OnceLock::new();
    CACHED
        .get_or_init(|| {
            let variant = std::env::current_exe()
                .ok()
                .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
                .and_then(|name| name.strip_prefix("voxtype-").map(str::to_string))
                .filter(|v| !v.is_empty());
            (
                variant.clone(),
                Some(backend_for(variant.as_deref()).to_string()),
            )
        })
        .clone()
}

fn backend_for(variant: Option<&str>) -> &'static str {
    let v = variant.unwrap_or_default();
    if v.contains("cuda") || cfg!(feature = "gpu-cuda") {
        "cuda"
    } else if v.contains("migraphx") {
        "migraphx"
    } else if v.contains("rocm") || cfg!(feature = "gpu-hipblas") {
        "rocm"
    } else if v.contains("vulkan") || cfg!(feature = "gpu-vulkan") {
        "vulkan"
    } else if cfg!(feature = "gpu-metal") {
        "metal"
    } else {
        "cpu"
    }
}

/// One line of settings, for the log. Never the excluded apps' names in
/// full detail beyond the list itself, and never any dictated text.
fn describe(s: &HistoryConfig) -> String {
    format!(
        "{}{}, keep {}",
        if s.enabled { "on" } else { "off" },
        if s.paused { " (paused)" } else { "" },
        if s.retention_days == 0 {
            "forever".to_string()
        } else {
            format!("{} days", s.retention_days)
        }
    )
}

/// Append the entry and index it. The index is best effort: the day file is
/// the record, and a missed row is picked up by the next search.
fn write<G>(
    store_dir: &Path,
    index_db: &Path,
    entry: &mut HistoryEntry,
    prepare: impl FnOnce(&mut HistoryEntry) -> std::io::Result<Option<G>>,
) -> Result<bool, String> {
    let store = HistoryStore::at(store_dir);
    if !store
        .append_prepared(entry, prepare)
        .map_err(|e| e.to_string())?
    {
        return Ok(false);
    }
    match HistoryIndex::open(index_db) {
        Ok(index) => {
            if let Err(e) = index.add(&store, entry) {
                tracing::debug!("History index not updated: {}", e);
            }
        }
        Err(e) => tracing::debug!("History index unavailable: {}", e),
    }
    Ok(true)
}

/// Retention and compression, then drop index rows for removed days.
pub fn maintenance(store_dir: &Path, index_db: &Path, retention_days: u32, today: NaiveDate) {
    if !store_dir.exists() {
        return;
    }
    let store = match HistoryStore::open(store_dir) {
        Ok(store) => store,
        Err(e) => {
            tracing::warn!("History maintenance skipped: {}", e);
            return;
        }
    };
    match store.prune(retention_days, today) {
        Ok(removed) if !removed.is_empty() => tracing::info!(
            "History: removed {} day(s) older than {} days",
            removed.len(),
            retention_days
        ),
        Ok(_) => {}
        Err(e) => tracing::warn!("History retention failed: {}", e),
    }
    if let Err(e) = store.compress_old(today) {
        tracing::warn!("Compressing old history failed: {}", e);
    }
    match HistoryIndex::open(index_db) {
        Ok(index) => {
            if let Err(e) = index.sync(&store) {
                tracing::debug!("History index sync failed: {}", e);
            }
        }
        Err(e) => tracing::debug!("History index unavailable: {}", e),
    }
}

/// Run maintenance now and then every day, for the daemon's lifetime.
pub fn spawn_daily_maintenance(recorder: Arc<Mutex<Recorder>>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let job = {
                let mut r = recorder.lock().unwrap_or_else(|e| e.into_inner());
                let retention = r.refresh().retention_days;
                let today = Local::now().date_naive();
                r.last_maintenance = Some(today);
                (r.store_dir.clone(), r.index_db.clone(), retention, today)
            };
            let _ = tokio::task::spawn_blocking(move || {
                let (dir, db, retention, today) = job;
                maintenance(&dir, &db, retention, today)
            })
            .await;
            tokio::time::sleep(std::time::Duration::from_secs(24 * 60 * 60)).await;
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recorder(settings: HistoryConfig) -> (tempfile::TempDir, Recorder) {
        let t = tempfile::tempdir().unwrap();
        let r = Recorder::with_paths(
            settings,
            t.path().join("history"),
            t.path().join("meetings").join("index.db"),
        );
        (t, r)
    }

    fn on() -> HistoryConfig {
        HistoryConfig {
            enabled: true,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn off_or_paused_writes_nothing() {
        let (t, mut r) = recorder(HistoryConfig::default());
        assert!(r.begin().is_none());
        let (_t2, mut r2) = recorder(HistoryConfig {
            paused: true,
            ..on()
        });
        assert!(r2.begin().is_none());
        assert!(!t.path().join("history").exists());
    }

    #[tokio::test]
    async fn a_delivered_dictation_is_written_and_searchable() {
        let (t, mut r) = recorder(on());
        let mut p = r.begin().unwrap();
        p.transcribing(16_000);
        p.transcribed();
        p.texts("um send the invoice", "Send the invoice.");
        p.outcome(Outcome::Delivered, None);
        let id = p.entry.id.clone();
        r.commit(p).unwrap().await.unwrap();

        let store = HistoryStore::at(t.path().join("history"));
        let e = store.find(&id).unwrap().unwrap();
        assert_eq!(e.text, "Send the invoice.");
        assert_eq!(e.audio_ms, 1000);
        assert!(e.delivered);
        assert!(!e.edits.is_empty());
        let index = HistoryIndex::open(&t.path().join("meetings").join("index.db")).unwrap();
        assert_eq!(index.count().unwrap(), 1);
    }

    #[tokio::test]
    async fn failures_and_cancels_are_recorded_too() {
        let (t, mut r) = recorder(on());
        let mut p = r.begin().unwrap();
        p.outcome(Outcome::Cancelled, None);
        // A later outcome does not overwrite the first.
        p.outcome(Outcome::Delivered, None);
        r.commit(p).unwrap().await.unwrap();

        let p = r.begin().unwrap();
        r.commit(p).unwrap().await.unwrap();

        let entries = HistoryStore::at(t.path().join("history")).all().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].outcome, Outcome::Cancelled);
        assert_eq!(entries[0].ended_by, EndedBy::Cancelled);
        assert!(!entries[0].delivered);
        assert_eq!(entries[1].outcome, Outcome::TranscriptionFailed);
    }

    fn window(app: &str, title: &str) -> Option<FocusedWindow> {
        Some(FocusedWindow {
            app: app.into(),
            title: title.into(),
        })
    }

    #[test]
    fn excluded_apps_are_never_written_whichever_end_they_are_at() {
        let s = on();
        let mut e = HistoryEntry::new(Local::now());
        assert!(!apply_context(
            &s,
            &mut e,
            window("org.keepassxc.KeePassXC", "x"),
            None
        ));
        assert!(!apply_context(
            &s,
            &mut e,
            window("kitty", "x"),
            window("Bitwarden", "")
        ));
        assert!(!apply_context(&s, &mut e, None, window("1Password", "")));
    }

    #[test]
    fn the_app_is_recorded_and_the_title_only_when_asked() {
        let mut e = HistoryEntry::new(Local::now());
        assert!(apply_context(
            &on(),
            &mut e,
            window("kitty", "start"),
            window("firefox", "Inbox (3)")
        ));
        assert_eq!(e.app.as_deref(), Some("firefox"));
        assert_eq!(e.window_title, None);

        let mut e = HistoryEntry::new(Local::now());
        let titles = HistoryConfig {
            record_window_title: true,
            record_app: false,
            ..on()
        };
        assert!(apply_context(
            &titles,
            &mut e,
            window("kitty", "~/src"),
            None
        ));
        assert_eq!(e.app, None);
        assert_eq!(e.window_title.as_deref(), Some("~/src"));

        // Unknown app (no supported compositor): recorded, app empty.
        let mut e = HistoryEntry::new(Local::now());
        assert!(apply_context(&on(), &mut e, None, None));
        assert_eq!(e.app, None);
    }

    #[test]
    fn backends_follow_the_variant_name() {
        if cfg!(any(
            feature = "gpu-cuda",
            feature = "gpu-hipblas",
            feature = "gpu-vulkan",
            feature = "gpu-metal"
        )) {
            return;
        }
        assert_eq!(backend_for(Some("onnx-cuda-13")), "cuda");
        assert_eq!(backend_for(Some("onnx-migraphx")), "migraphx");
        assert_eq!(backend_for(Some("vulkan")), "vulkan");
        assert_eq!(backend_for(Some("avx2")), "cpu");
        assert_eq!(backend_for(None), "cpu");
    }

    #[test]
    fn describe_never_mentions_text() {
        assert_eq!(describe(&on()), "on, keep 30 days");
        assert_eq!(
            describe(&HistoryConfig {
                paused: true,
                retention_days: 0,
                ..on()
            }),
            "on (paused), keep forever"
        );
    }

    #[tokio::test]
    async fn pausing_in_the_config_file_applies_to_the_next_dictation() {
        let t = tempfile::tempdir().unwrap();
        let cfg_path = t.path().join("config.toml");
        std::fs::write(&cfg_path, "[history]\nenabled = true\n").unwrap();
        let config = crate::config::load_config(Some(&cfg_path)).unwrap();
        let mut r = Recorder::new(&config, Some(cfg_path.clone()));
        r.store_dir = t.path().join("history");
        r.index_db = t.path().join("index.db");
        assert!(r.begin().is_some());

        // A different size guarantees the change is seen even within one
        // mtime tick.
        std::fs::write(&cfg_path, "[history]\nenabled = true\npaused = true\n").unwrap();
        assert!(r.begin().is_none());
        std::fs::write(
            &cfg_path,
            "[history]\nenabled = true\npaused = false # resumed\n",
        )
        .unwrap();
        assert!(r.begin().is_some());
    }

    #[tokio::test]
    async fn pausing_mid_dictation_drops_it() {
        let t = tempfile::tempdir().unwrap();
        let cfg_path = t.path().join("config.toml");
        std::fs::write(&cfg_path, "[history]\nenabled = true\n").unwrap();
        let config = crate::config::load_config(Some(&cfg_path)).unwrap();
        let mut r = Recorder::new(&config, Some(cfg_path.clone()));
        r.store_dir = t.path().join("history");
        r.index_db = t.path().join("index.db");
        let mut p = r.begin().unwrap();
        p.outcome(Outcome::Delivered, None);
        std::fs::write(&cfg_path, "[history]\nenabled = true\npaused = true\n").unwrap();
        assert!(r.commit(p).is_none());
        assert!(!t.path().join("history").exists());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn pausing_after_commit_before_the_queued_writer_runs_discards_the_entry() {
        let t = tempfile::tempdir().unwrap();
        let path = t.path().join("config.toml");
        std::fs::write(&path, "[history]\nenabled = true\n").unwrap();
        let config = crate::config::load_config(Some(&path)).unwrap();
        let mut r = Recorder::new(&config, Some(path.clone()));
        r.store_dir = t.path().join("history");
        r.index_db = t.path().join("index.db");
        let mut p = r.begin().unwrap();
        p.texts("synthetic sensitive words", "Synthetic sensitive words.");
        p.outcome(Outcome::Delivered, None);
        let writer = r.commit(p).unwrap();
        std::fs::write(&path, "[history]\nenabled = true\npaused = true\n").unwrap();
        writer.await.unwrap();
        assert!(!r
            .store_dir
            .join(format!("{}.jsonl", chrono::Local::now().date_naive()))
            .exists());
        assert!(!r.index_db.exists());
    }

    async fn privacy_change_while_writer_waits_on_storage_lock(
        setting: &str,
        value: &str,
        resume: bool,
    ) {
        let t = tempfile::tempdir().unwrap();
        let path = t.path().join("config.toml");
        std::fs::write(&path, "[history]\nenabled = true\n").unwrap();
        let config = crate::config::load_config(Some(&path)).unwrap();
        let mut r = Recorder::new(&config, Some(path.clone()));
        r.store_dir = t.path().join("history");
        r.index_db = t.path().join("index.db");
        HistoryStore::open(&r.store_dir).unwrap();
        let held = super::super::store::DirLock::take(&r.store_dir).unwrap();
        let waiting = super::super::store::observe_next_lock(&r.store_dir);
        let mut pending = r.begin().unwrap();
        pending.texts(
            "synthetic lock-boundary text",
            "Synthetic lock-boundary text.",
        );
        pending.outcome(Outcome::Delivered, None);
        let writer = r.commit(pending).unwrap();
        tokio::task::spawn_blocking(move || {
            waiting
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap()
        })
        .await
        .unwrap();
        // The real writer has reached flock while this test holds its lock.
        // Persist the privacy change before permitting the append to proceed.
        crate::config_set::set_key(path.clone(), setting, value).unwrap();
        if resume {
            crate::config_set::set_key(path, setting, "false").unwrap();
        }
        drop(held);
        tokio::time::timeout(std::time::Duration::from_secs(5), writer)
            .await
            .unwrap()
            .unwrap();
        assert!(
            HistoryStore::at(&r.store_dir).all().unwrap().is_empty(),
            "writer retained a dictation after persisted {setting}"
        );
        assert!(!r.index_db.exists());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn privacy_change_storage_lock_pause_discards_entry() {
        privacy_change_while_writer_waits_on_storage_lock("history.paused", "true", false).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn privacy_change_storage_lock_disable_discards_entry() {
        privacy_change_while_writer_waits_on_storage_lock("history.enabled", "false", false).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn privacy_change_storage_lock_resume_does_not_resurrect_a_paused_dictation() {
        privacy_change_while_writer_waits_on_storage_lock("history.paused", "true", true).await;
    }
}
