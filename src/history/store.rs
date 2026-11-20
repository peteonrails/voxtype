//! The history files: one JSON line per dictation, one file per local day.
//!
//! ```text
//! ~/.local/share/voxtype/history/            0700
//!   2026-11-14.jsonl                         0600, today, appended to
//!   2026-11-01.jsonl.gz                      0600, older than a week
//!   .lock                                    flock(2) between writers
//! ```
//!
//! The files are the record; the search index is derived from them and can
//! always be rebuilt. Changing one entry (a delete or a correction) rewrites
//! its whole day file atomically: write a temporary file, fsync it, rename it
//! over the old one, fsync the directory. A crash before the rename leaves the
//! old file intact and a stray temporary file, which the next open removes.
//! Retention removes whole day files.

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::fs::{DirBuilderExt, FileExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use chrono::{Days, NaiveDate};
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;

use super::entry::{day_of_id, HistoryEntry};

/// Day files older than this many days are gzipped.
pub const COMPRESS_AFTER_DAYS: u64 = 7;

const DIR_MODE: u32 = 0o700;
const FILE_MODE: u32 = 0o600;
const TEMP_PREFIX: &str = ".tmp-";

/// One day's file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DayFile {
    pub date: NaiveDate,
    pub path: PathBuf,
    pub gzipped: bool,
}

/// What reading a day file found.
#[derive(Debug, Default)]
pub struct DayRead {
    /// Entries in file order (oldest first).
    pub entries: Vec<HistoryEntry>,
    /// Lines that did not parse. They are skipped when reading and kept
    /// untouched when the file is rewritten.
    pub bad_lines: usize,
}

/// The history directory.
#[derive(Debug, Clone)]
pub struct HistoryStore {
    dir: PathBuf,
}

/// `YYYY-MM-DD` from a day file name, and whether it is gzipped.
fn parse_day_name(name: &str) -> Option<(NaiveDate, bool)> {
    let (stem, gzipped) = match name.strip_suffix(".jsonl.gz") {
        Some(stem) => (stem, true),
        None => (name.strip_suffix(".jsonl")?, false),
    };
    let date = NaiveDate::parse_from_str(stem, "%Y-%m-%d").ok()?;
    // Reject names chrono accepts loosely, such as "2026-1-5".
    (date.format("%Y-%m-%d").to_string() == stem).then_some((date, gzipped))
}

fn day_name(date: NaiveDate, gzipped: bool) -> String {
    format!(
        "{}.jsonl{}",
        date.format("%Y-%m-%d"),
        if gzipped { ".gz" } else { "" }
    )
}

/// Open a file for writing, created `0600`.
fn create_private(path: &Path, append: bool) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options
        .read(true)
        .mode(FILE_MODE)
        .create(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK);
    if append {
        options.append(true);
    } else {
        options.write(true).truncate(true);
    }
    let file = options.open(path)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "History requires a regular file",
        ));
    }
    // `mode` only applies when the file is created; tighten an older one too.
    let perms = file.metadata()?.permissions();
    if perms.mode() & 0o777 != FILE_MODE {
        file.set_permissions(fs::Permissions::from_mode(FILE_MODE))?;
    }
    Ok(file)
}

/// An exclusive advisory lock on the directory, held while writing.
pub(crate) struct DirLock {
    _file: File,
}

impl DirLock {
    pub(super) fn take(dir: &Path) -> io::Result<Self> {
        use std::os::fd::AsRawFd;
        let file = create_private(&dir.join(".lock"), true)?;
        #[cfg(test)]
        {
            let mut observers = LOCK_OBSERVERS.lock().unwrap();
            if let Some(position) = observers.iter().position(|(path, _)| path == dir) {
                let (_, sender) = observers.remove(position);
                let _ = sender.send(());
            }
        }
        // SAFETY: flock on a file descriptor we own; it is released when the
        // descriptor is closed, which is when `_file` drops.
        let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
        if rc != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self { _file: file })
    }
}

fn sync_dir(dir: &Path) -> io::Result<()> {
    File::open(dir)?.sync_all()
}

/// Serialize a config's atomic save with the final history append decision.
/// Writers take the day-store lock first, then this short-lived control lock.
/// Config saves take only this lock: a busy store cannot delay a pause save.
pub(crate) fn privacy_lock(config_path: &Path) -> io::Result<DirLock> {
    let dir = privacy_dir(config_path)?;
    ensure_private_dir(&dir)?;
    DirLock::take(&dir)
}

fn privacy_dir(config_path: &Path) -> io::Result<PathBuf> {
    use sha2::{Digest, Sha256};
    let parent = config_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let name = config_path
        .file_name()
        .ok_or_else(|| io::Error::other("config path has no filename"))?;
    let identity = parent.canonicalize()?.join(name);
    let key = format!(
        "{:x}",
        Sha256::digest(identity.as_os_str().as_encoded_bytes())
    );
    // Keep locks writable even when the selected config is system-owned.
    Ok(control_root(&identity).join("history-control").join(key))
}

/// Where the privacy control files live: the data dir, which the daemon and
/// the CLI both resolve the same way.
#[cfg(not(test))]
fn control_root(_config: &Path) -> PathBuf {
    crate::config::Config::data_dir()
}

/// Tests never use the real data dir, which may be unwritable (the Nix build
/// sandbox) or belong to the developer. A test's config lives in its own
/// temporary directory, and that directory stands in for the data dir unless
/// the test chose one with [`use_data_dir_for`].
#[cfg(test)]
fn control_root(config: &Path) -> PathBuf {
    TEST_DATA_DIRS
        .lock()
        .unwrap()
        .iter()
        .find(|(identity, _)| identity == config)
        .map(|(_, dir)| dir.clone())
        .unwrap_or_else(|| config.parent().unwrap_or(Path::new(".")).to_path_buf())
}

#[cfg(test)]
static TEST_DATA_DIRS: std::sync::Mutex<Vec<(PathBuf, PathBuf)>> =
    std::sync::Mutex::new(Vec::new());

/// Tests: keep the privacy control files for `config_path` under `data_dir`.
#[cfg(test)]
pub(crate) fn use_data_dir_for(config_path: &Path, data_dir: &Path) {
    let parent = config_path.parent().unwrap().canonicalize().unwrap();
    let identity = parent.join(config_path.file_name().unwrap());
    let mut dirs = TEST_DATA_DIRS.lock().unwrap();
    dirs.retain(|(known, _)| known != &identity);
    dirs.push((identity, data_dir.to_path_buf()));
}

/// Call only while holding this config's privacy lock.
pub(crate) fn privacy_generation(config_path: &Path) -> io::Result<u64> {
    let path = privacy_dir(config_path)?.join("generation");
    let mut file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error),
    };
    if !file.metadata()?.is_file() || file.metadata()?.len() != 8 {
        return Err(io::Error::other(
            "Invalid history privacy generation; history is not being recorded",
        ));
    }
    let mut bytes = [0; 8];
    file.read_exact(&mut bytes)?;
    Ok(u64::from_le_bytes(bytes))
}

/// Invalidate in-flight sessions before persisting a pause/disable. If a
/// subsequent config save fails, discarding those sessions is privacy-safe.
/// Call only while holding this config's privacy lock.
pub(crate) fn advance_privacy_generation(config_path: &Path) -> io::Result<()> {
    let next = privacy_generation(config_path)?
        .checked_add(1)
        .ok_or_else(|| io::Error::other("History privacy generation exhausted"))?;
    let dir = privacy_dir(config_path)?;
    let mut file = tempfile::Builder::new()
        .prefix(".tmp-generation-")
        .tempfile_in(&dir)?;
    file.as_file()
        .set_permissions(fs::Permissions::from_mode(FILE_MODE))?;
    file.write_all(&next.to_le_bytes())?;
    file.as_file().sync_all()?;
    file.persist(dir.join("generation"))
        .map_err(|error| error.error)?;
    sync_dir(&dir)
}

#[cfg(test)]
static LOCK_OBSERVERS: std::sync::Mutex<Vec<(PathBuf, std::sync::mpsc::Sender<()>)>> =
    std::sync::Mutex::new(Vec::new());

#[cfg(test)]
pub(super) fn observe_next_lock(dir: &Path) -> std::sync::mpsc::Receiver<()> {
    let (sender, receiver) = std::sync::mpsc::channel();
    LOCK_OBSERVERS
        .lock()
        .unwrap()
        .push((dir.to_path_buf(), sender));
    receiver
}

/// Create private directories without following pre-planted directory links.
/// Existing ancestors are inspected but never chmodded.
pub(crate) fn ensure_private_dir(dir: &Path) -> io::Result<()> {
    let absolute = if dir.is_absolute() {
        dir.to_path_buf()
    } else {
        std::env::current_dir()?.join(dir)
    };
    let mut current = PathBuf::new();
    for component in absolute.components() {
        if component == std::path::Component::ParentDir {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Use a history path without parent traversal",
            ));
        }
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => {}
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "History directories must not be symbolic links or files",
                ))
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                match fs::DirBuilder::new().mode(DIR_MODE).create(&current) {
                    Ok(()) => {}
                    Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                        let meta = fs::symlink_metadata(&current)?;
                        if !meta.is_dir() || meta.file_type().is_symlink() {
                            return Err(e);
                        }
                    }
                    Err(e) => return Err(e),
                }
            }
            Err(e) => return Err(e),
        }
    }
    let pinned = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&absolute)?;
    pinned.set_permissions(fs::Permissions::from_mode(DIR_MODE))
}

impl HistoryStore {
    /// `~/.local/share/voxtype/history`.
    pub fn default_dir() -> PathBuf {
        crate::config::Config::data_dir().join("history")
    }

    /// The store at `dir`, without touching the disk. Reads of a directory
    /// that does not exist find nothing.
    pub fn at(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// The store at `dir`, created `0700` if missing (and tightened to `0700`
    /// if not). Removes temporary files a crash left behind.
    pub fn open(dir: impl Into<PathBuf>) -> io::Result<Self> {
        let store = Self::at(dir);
        store.ensure_dir()?;
        let _lock = DirLock::take(&store.dir)?;
        store.remove_stale_temps();
        Ok(store)
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn ensure_dir(&self) -> io::Result<()> {
        ensure_private_dir(&self.dir)
    }

    fn remove_stale_temps(&self) {
        let Ok(rd) = fs::read_dir(&self.dir) else {
            return;
        };
        for entry in rd.flatten() {
            if entry.file_name().to_string_lossy().starts_with(TEMP_PREFIX) {
                let _ = fs::remove_file(entry.path());
            }
        }
    }

    /// Every day file, oldest first. A day with both a plain and a gzipped
    /// file (a crash between writing the `.gz` and removing the plain one)
    /// lists the plain file, which is the complete one.
    pub fn days(&self) -> io::Result<Vec<DayFile>> {
        match fs::symlink_metadata(&self.dir) {
            Ok(_) => self.ensure_dir()?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e),
        }
        let rd = match fs::read_dir(&self.dir) {
            Ok(rd) => rd,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        let mut days: std::collections::BTreeMap<NaiveDate, DayFile> = Default::default();
        for entry in rd.flatten() {
            let name = entry.file_name();
            let Some((date, gzipped)) = parse_day_name(&name.to_string_lossy()) else {
                continue;
            };
            let file = DayFile {
                date,
                path: entry.path(),
                gzipped,
            };
            match days.get(&date) {
                Some(existing) if !existing.gzipped => {}
                _ => {
                    days.insert(date, file);
                }
            }
        }
        Ok(days.into_values().collect())
    }

    fn day_file(&self, date: NaiveDate) -> io::Result<Option<DayFile>> {
        Ok(self.days()?.into_iter().find(|d| d.date == date))
    }

    /// The raw lines of a day file.
    fn read_lines(file: &DayFile) -> io::Result<Vec<String>> {
        let raw = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
            .open(&file.path)?;
        if !raw.metadata()?.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "History requires a regular day file",
            ));
        }
        let reader: Box<dyn Read> = if file.gzipped {
            Box::new(GzDecoder::new(raw))
        } else {
            Box::new(raw)
        };
        let mut lines = Vec::new();
        for line in BufReader::new(reader).lines() {
            let line = line?;
            if !line.trim().is_empty() {
                lines.push(line);
            }
        }
        Ok(lines)
    }

    /// Read one day file.
    pub fn read_day_file(file: &DayFile) -> io::Result<DayRead> {
        let mut read = DayRead::default();
        for line in Self::read_lines(file)? {
            match serde_json::from_str::<HistoryEntry>(&line) {
                Ok(entry) => read.entries.push(entry),
                Err(_) => read.bad_lines += 1,
            }
        }
        if read.bad_lines > 0 {
            tracing::warn!(
                "Skipped {} unreadable line(s) in {}",
                read.bad_lines,
                file.path.display()
            );
        }
        Ok(read)
    }

    /// Read one day. A day with no file reads as empty.
    pub fn read_day(&self, date: NaiveDate) -> io::Result<DayRead> {
        match self.day_file(date)? {
            Some(file) => Self::read_day_file(&file),
            None => Ok(DayRead::default()),
        }
    }

    /// Append an entry to its day file.
    pub fn append(&self, entry: &HistoryEntry) -> io::Result<()> {
        self.ensure_dir()?;
        let _lock = DirLock::take(&self.dir)?;
        self.append_locked(entry)
    }

    /// Prepare under the storage lock; keep the returned guard through fsync.
    /// A rejected entry never reaches serialization or a day file.
    pub(crate) fn append_prepared<G>(
        &self,
        entry: &mut HistoryEntry,
        prepare: impl FnOnce(&mut HistoryEntry) -> io::Result<Option<G>>,
    ) -> io::Result<bool> {
        self.ensure_dir()?;
        let _lock = DirLock::take(&self.dir)?;
        let Some(_privacy) = prepare(entry)? else {
            return Ok(false);
        };
        self.remove_stale_temps();
        self.append_locked(entry)?;
        Ok(true)
    }

    fn append_locked(&self, entry: &HistoryEntry) -> io::Result<()> {
        let date = entry
            .day()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "malformed entry id"))?;
        let mut line = serde_json::to_string(entry).map_err(io::Error::other)?;
        line.push('\n');
        match self.day_file(date)? {
            // An old day that is already compressed: rewrite it with the new
            // line. Only happens for an entry dated in the past, such as one
            // imported or restored.
            Some(file) if file.gzipped => {
                let mut lines = Self::read_lines(&file)?;
                lines.push(line.trim_end().to_string());
                self.write_day(date, true, &lines)
            }
            _ => {
                let path = self.dir.join(day_name(date, false));
                let mut file = create_private(&path, true)?;
                let length = file.metadata()?.len();
                if length > 0 {
                    let mut last = [0u8];
                    file.read_exact_at(&mut last, length - 1)?;
                    if last[0] != b'\n' {
                        // A crash may leave either valid JSON or a fragment
                        // without its newline. Never glue the next entry to it.
                        file.write_all(b"\n")?;
                    }
                }
                file.write_all(line.as_bytes())?;
                file.sync_data()
            }
        }
    }

    /// Replace a day's file with `lines`, atomically. No lines removes it.
    fn write_day(&self, date: NaiveDate, gzipped: bool, lines: &[String]) -> io::Result<()> {
        let target = self.dir.join(day_name(date, gzipped));
        let other = self.dir.join(day_name(date, !gzipped));
        if lines.is_empty() {
            for path in [&target, &other] {
                match fs::remove_file(path) {
                    Ok(()) => {}
                    Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e),
                }
            }
            return sync_dir(&self.dir);
        }
        let mut temp = tempfile::Builder::new()
            .prefix(TEMP_PREFIX)
            .tempfile_in(&self.dir)?;
        temp.as_file()
            .set_permissions(fs::Permissions::from_mode(FILE_MODE))?;
        let mut body = String::new();
        for line in lines {
            body.push_str(line);
            body.push('\n');
        }
        if gzipped {
            let mut gz = GzEncoder::new(temp.as_file_mut(), flate2::Compression::default());
            gz.write_all(body.as_bytes())?;
            gz.finish()?;
        } else {
            temp.as_file_mut().write_all(body.as_bytes())?;
        }
        temp.as_file().sync_all()?;
        temp.persist(&target).map_err(|e| e.error)?;
        match fs::remove_file(&other) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        sync_dir(&self.dir)
    }

    /// Rewrite the day an entry lives in, applying `change` to the line with
    /// that ID. `change` returns `None` to remove the entry. Lines that do not
    /// parse are kept as they were. Returns the entry as it was before the
    /// change, or `None` when no entry has that ID.
    fn rewrite_entry(
        &self,
        id: &str,
        change: impl FnOnce(HistoryEntry) -> Option<HistoryEntry>,
    ) -> io::Result<Option<HistoryEntry>> {
        let Some(date) = day_of_id(id) else {
            return Ok(None);
        };
        if !self.dir.exists() {
            return Ok(None);
        }
        self.ensure_dir()?;
        let _lock = DirLock::take(&self.dir)?;
        let Some(file) = self.day_file(date)? else {
            return Ok(None);
        };
        let mut lines = Self::read_lines(&file)?;
        let found = lines.iter().position(|line| {
            serde_json::from_str::<HistoryEntry>(line).is_ok_and(|entry| entry.id == id)
        });
        let Some(index) = found else {
            return Ok(None);
        };
        let before: HistoryEntry = serde_json::from_str(&lines[index]).map_err(io::Error::other)?;
        match change(before.clone()) {
            Some(after) => {
                if before.schema != super::entry::SCHEMA_VERSION {
                    return Err(io::Error::new(io::ErrorKind::Unsupported, "This history entry uses another schema version. Upgrade Voxtype before correcting it; its original file has not been changed."));
                }
                lines[index] = serde_json::to_string(&after).map_err(io::Error::other)?
            }
            None => {
                lines.remove(index);
            }
        }
        self.write_day(date, file.gzipped, &lines)?;
        Ok(Some(before))
    }

    /// Find an entry by ID.
    pub fn find(&self, id: &str) -> io::Result<Option<HistoryEntry>> {
        let Some(date) = day_of_id(id) else {
            return Ok(None);
        };
        Ok(self
            .read_day(date)?
            .entries
            .into_iter()
            .find(|entry| entry.id == id))
    }

    /// Change an entry in place. Returns the entry after the change, or
    /// `None` when no entry has that ID.
    pub fn update(
        &self,
        id: &str,
        change: impl FnOnce(&mut HistoryEntry),
    ) -> io::Result<Option<HistoryEntry>> {
        let mut after = None;
        self.rewrite_entry(id, |mut entry| {
            change(&mut entry);
            after = Some(entry.clone());
            Some(entry)
        })?;
        Ok(after)
    }

    /// Delete an entry. Returns it, or `None` when no entry has that ID.
    pub fn delete(&self, id: &str) -> io::Result<Option<HistoryEntry>> {
        self.rewrite_entry(id, |_| None)
    }

    /// The most recent entries, newest first.
    pub fn recent(&self, limit: usize) -> io::Result<Vec<HistoryEntry>> {
        let mut out = Vec::new();
        for day in self.days()?.into_iter().rev() {
            let mut entries = Self::read_day_file(&day)?.entries;
            entries.sort_by_key(|e| std::cmp::Reverse(e.ts_start));
            for entry in entries {
                if out.len() >= limit {
                    return Ok(out);
                }
                out.push(entry);
            }
        }
        Ok(out)
    }

    /// Every entry, oldest first.
    pub fn all(&self) -> io::Result<Vec<HistoryEntry>> {
        let mut out = Vec::new();
        for day in self.days()? {
            out.extend(Self::read_day_file(&day)?.entries);
        }
        Ok(out)
    }

    /// Remove whole days: every day before `before`, or every day when
    /// `before` is `None`. Returns the days removed.
    pub fn clear(&self, before: Option<NaiveDate>) -> io::Result<Vec<NaiveDate>> {
        if !self.dir.exists() {
            return Ok(Vec::new());
        }
        self.ensure_dir()?;
        let _lock = DirLock::take(&self.dir)?;
        let mut removed = Vec::new();
        for day in self.days()? {
            if before.is_none_or(|b| day.date < b) {
                // A day can have both a plain and a gzipped file after a
                // crash; remove both.
                for gz in [false, true] {
                    match fs::remove_file(self.dir.join(day_name(day.date, gz))) {
                        Ok(()) => {}
                        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                        Err(e) => return Err(e),
                    }
                }
                removed.push(day.date);
            }
        }
        if !removed.is_empty() {
            sync_dir(&self.dir)?;
        }
        Ok(removed)
    }

    /// Remove days older than `retention_days` (counting `today` as the
    /// first). `0` keeps everything. Returns the days removed.
    pub fn prune(&self, retention_days: u32, today: NaiveDate) -> io::Result<Vec<NaiveDate>> {
        if retention_days == 0 {
            return Ok(Vec::new());
        }
        let Some(first_kept) = today.checked_sub_days(Days::new(u64::from(retention_days) - 1))
        else {
            return Ok(Vec::new());
        };
        self.clear(Some(first_kept))
    }

    /// Gzip the plain day files from before the last week. Returns how many
    /// were compressed.
    pub fn compress_old(&self, today: NaiveDate) -> io::Result<usize> {
        let Some(cutoff) = today.checked_sub_days(Days::new(COMPRESS_AFTER_DAYS)) else {
            return Ok(0);
        };
        if !self.dir.exists() {
            return Ok(0);
        }
        self.ensure_dir()?;
        let _lock = DirLock::take(&self.dir)?;
        let mut done = 0;
        for day in self.days()? {
            if day.gzipped || day.date > cutoff {
                continue;
            }
            let lines = Self::read_lines(&day)?;
            self.write_day(day.date, true, &lines)?;
            done += 1;
        }
        Ok(done)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::entry::Outcome;
    use chrono::{Local, TimeZone};

    fn store() -> (tempfile::TempDir, HistoryStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = HistoryStore::open(dir.path().join("history")).unwrap();
        (dir, store)
    }

    fn entry_on(y: i32, m: u32, d: u32, h: u32, text: &str) -> HistoryEntry {
        let mut e = HistoryEntry::new(Local.with_ymd_and_hms(y, m, d, h, 0, 0).unwrap());
        e.raw_text = text.to_string();
        e.text = text.to_string();
        e.set_outcome(Outcome::Delivered, None);
        e
    }

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn mode(path: &Path) -> u32 {
        fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn appends_one_private_line_per_dictation_in_a_private_dir() {
        let (_t, store) = store();
        let a = entry_on(2026, 11, 14, 9, "first");
        let b = entry_on(2026, 11, 14, 10, "second");
        store.append(&a).unwrap();
        store.append(&b).unwrap();

        let path = store.dir().join("2026-11-14.jsonl");
        let body = fs::read_to_string(&path).unwrap();
        assert_eq!(body.lines().count(), 2);
        assert!(body.ends_with('\n'));
        assert_eq!(mode(store.dir()), 0o700);
        assert_eq!(mode(&path), 0o600);
        assert_eq!(mode(&store.dir().join(".lock")), 0o600);

        let read = store.read_day(date(2026, 11, 14)).unwrap();
        assert_eq!(read.entries, vec![a.clone(), b.clone()]);
        assert_eq!(store.find(&b.id).unwrap(), Some(b));
    }

    #[test]
    fn opening_tightens_a_loose_directory() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("history");
        fs::create_dir(&dir).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
        HistoryStore::open(&dir).unwrap();
        assert_eq!(mode(&dir), 0o700);
    }

    #[test]
    fn delete_and_update_rewrite_only_that_entry() {
        let (_t, store) = store();
        let a = entry_on(2026, 11, 14, 9, "keep me");
        let b = entry_on(2026, 11, 14, 10, "secret");
        store.append(&a).unwrap();
        store.append(&b).unwrap();

        let fixed = store
            .update(&a.id, |e| e.corrected_text = Some("Keep me.".into()))
            .unwrap()
            .unwrap();
        assert_eq!(fixed.corrected_text.as_deref(), Some("Keep me."));

        assert_eq!(store.delete(&b.id).unwrap().unwrap().text, "secret");
        assert_eq!(store.delete(&b.id).unwrap(), None);
        let path = store.dir().join("2026-11-14.jsonl");
        let body = fs::read_to_string(&path).unwrap();
        assert!(!body.contains("secret"));
        assert!(body.contains("Keep me."));
        assert_eq!(mode(&path), 0o600);

        // Deleting the last entry removes the file.
        store.delete(&a.id).unwrap();
        assert!(!path.exists());
        assert_eq!(
            store.update("20261114-000000-abcdef", |_| {}).unwrap(),
            None
        );
    }

    #[test]
    fn a_crash_before_the_rename_leaves_the_old_file_and_no_temp_after_reopen() {
        let (_t, store) = store();
        let a = entry_on(2026, 11, 14, 9, "survives");
        store.append(&a).unwrap();
        // What a crash mid-rewrite leaves: a half-written temporary file.
        let temp = store.dir().join(".tmp-2026-11-14.jsonl-999");
        fs::write(&temp, "{\"half\": ").unwrap();

        // Readers never see it...
        assert_eq!(store.read_day(date(2026, 11, 14)).unwrap().entries, vec![a]);
        assert_eq!(store.days().unwrap().len(), 1);
        // ...and the next open removes it.
        HistoryStore::open(store.dir()).unwrap();
        assert!(!temp.exists());
    }

    #[test]
    fn unreadable_lines_are_skipped_but_kept_on_rewrite() {
        let (_t, store) = store();
        let a = entry_on(2026, 11, 14, 9, "one");
        let b = entry_on(2026, 11, 14, 10, "two");
        store.append(&a).unwrap();
        let path = store.dir().join("2026-11-14.jsonl");
        let mut f = OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(f, "{{not json").unwrap();
        drop(f);
        store.append(&b).unwrap();

        let read = store.read_day(date(2026, 11, 14)).unwrap();
        assert_eq!(read.entries.len(), 2);
        assert_eq!(read.bad_lines, 1);

        store.delete(&a.id).unwrap();
        let body = fs::read_to_string(&path).unwrap();
        assert!(body.contains("{not json"));
        assert_eq!(store.read_day(date(2026, 11, 14)).unwrap().entries, vec![b]);
    }

    #[test]
    fn old_days_are_gzipped_and_still_readable_and_editable() {
        let (_t, store) = store();
        let old = entry_on(2026, 11, 1, 9, "old words");
        let recent = entry_on(2026, 11, 10, 9, "recent words");
        store.append(&old).unwrap();
        store.append(&recent).unwrap();

        assert_eq!(store.compress_old(date(2026, 11, 14)).unwrap(), 1);
        assert!(!store.dir().join("2026-11-01.jsonl").exists());
        let gz = store.dir().join("2026-11-01.jsonl.gz");
        assert!(gz.exists());
        assert_eq!(mode(&gz), 0o600);
        assert!(store.dir().join("2026-11-10.jsonl").exists());
        // Idempotent.
        assert_eq!(store.compress_old(date(2026, 11, 14)).unwrap(), 0);

        // zgrep-compatible: a plain gzip stream of the same lines.
        let mut text = String::new();
        GzDecoder::new(File::open(&gz).unwrap())
            .read_to_string(&mut text)
            .unwrap();
        assert!(text.contains("old words"));

        assert_eq!(store.find(&old.id).unwrap(), Some(old.clone()));
        store
            .update(&old.id, |e| e.corrected_text = Some("Old words.".into()))
            .unwrap();
        assert!(gz.exists());
        assert_eq!(
            store
                .find(&old.id)
                .unwrap()
                .unwrap()
                .corrected_text
                .as_deref(),
            Some("Old words.")
        );
        let recent_first: Vec<_> = store
            .recent(10)
            .unwrap()
            .into_iter()
            .map(|e| e.id)
            .collect();
        assert_eq!(recent_first, vec![recent.id, old.id]);
    }

    #[test]
    fn a_day_left_both_plain_and_gzipped_reads_the_plain_file() {
        let (_t, store) = store();
        let a = entry_on(2026, 11, 1, 9, "complete");
        store.append(&a).unwrap();
        let plain = store.dir().join("2026-11-01.jsonl");
        let gz = store.dir().join("2026-11-01.jsonl.gz");
        let mut enc = GzEncoder::new(File::create(&gz).unwrap(), Default::default());
        enc.write_all(b"").unwrap();
        enc.finish().unwrap();

        assert_eq!(store.days().unwrap()[0].path, plain);
        assert_eq!(store.find(&a.id).unwrap(), Some(a));
        store.compress_old(date(2026, 11, 14)).unwrap();
        assert!(!plain.exists());
        assert_eq!(store.all().unwrap().len(), 1);
    }

    #[test]
    fn retention_removes_whole_days_and_zero_keeps_everything() {
        let (_t, store) = store();
        for d in [1, 14, 15, 16] {
            store.append(&entry_on(2026, 10, d, 9, "x")).unwrap();
        }
        store.append(&entry_on(2026, 11, 14, 9, "today")).unwrap();
        store.compress_old(date(2026, 11, 14)).unwrap();

        assert!(store.prune(0, date(2026, 11, 14)).unwrap().is_empty());
        // 30 days counting today keeps 2026-10-16 onwards.
        let removed = store.prune(30, date(2026, 11, 14)).unwrap();
        assert_eq!(
            removed,
            vec![date(2026, 10, 1), date(2026, 10, 14), date(2026, 10, 15)]
        );
        let left: Vec<_> = store.days().unwrap().into_iter().map(|d| d.date).collect();
        assert_eq!(left, vec![date(2026, 10, 16), date(2026, 11, 14)]);
        // A retention of one keeps today only.
        store.prune(1, date(2026, 11, 14)).unwrap();
        assert_eq!(store.days().unwrap().len(), 1);
    }

    #[test]
    fn clear_takes_everything_or_the_days_before_a_date() {
        let (_t, store) = store();
        store.append(&entry_on(2026, 11, 1, 9, "a")).unwrap();
        store.append(&entry_on(2026, 11, 2, 9, "b")).unwrap();
        assert_eq!(
            store.clear(Some(date(2026, 11, 2))).unwrap(),
            vec![date(2026, 11, 1)]
        );
        assert_eq!(store.clear(None).unwrap(), vec![date(2026, 11, 2)]);
        assert!(store.all().unwrap().is_empty());
    }

    #[test]
    fn a_missing_directory_reads_as_empty_and_is_not_created() {
        let t = tempfile::tempdir().unwrap();
        let store = HistoryStore::at(t.path().join("nope"));
        assert!(store.recent(5).unwrap().is_empty());
        assert_eq!(store.find("20261114-000000-abcdef").unwrap(), None);
        assert_eq!(store.delete("20261114-000000-abcdef").unwrap(), None);
        assert!(store.clear(None).unwrap().is_empty());
        assert!(!t.path().join("nope").exists());
    }

    #[test]
    fn day_names_are_strict() {
        assert_eq!(
            parse_day_name("2026-11-14.jsonl"),
            Some((date(2026, 11, 14), false))
        );
        assert_eq!(
            parse_day_name("2026-11-14.jsonl.gz"),
            Some((date(2026, 11, 14), true))
        );
        for bad in [
            "2026-1-4.jsonl",
            ".tmp-2026-11-14.jsonl-1",
            "notes.txt",
            ".lock",
        ] {
            assert_eq!(parse_day_name(bad), None, "{bad}");
        }
    }
}
