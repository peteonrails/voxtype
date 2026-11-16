//! Full-text search over dictation history.
//!
//! A contentless FTS5 table (`content=''`, `contentless_delete=1`) in the
//! meetings `index.db`, next to `meeting_search`, so one database answers
//! both. Contentless means the table keeps only the search terms, never a
//! copy of the text: results read their text back from the day files by
//! entry ID. Each row's text is the entry's output text, its correction and
//! what the engine heard, when those differ.
//!
//! The index is derived. `history_search_days` remembers each day file's
//! size and modification time; [`HistoryIndex::sync`] reindexes any day whose
//! file changed, drops days whose file is gone, and indexes days it has never
//! seen, so a rebuild is only deleting `index.db`'s history tables.
//!
//! Deleted text must not linger: the connection runs with
//! `PRAGMA secure_delete = ON`, which zeroes freed pages, and every removal is
//! followed by an FTS5 `optimize`, which rewrites the term segments without
//! the deleted rows instead of leaving tombstones over the old terms.
//! WAL cleanup is checked after commit. A reader that pins old WAL frames
//! produces an explicit error; a later sync retries the checkpoint even
//! when no day fingerprints changed.

use std::collections::HashMap;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use chrono::NaiveDate;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use super::entry::HistoryEntry;
use super::store::{DayFile, HistoryStore};

/// Bumped when what is indexed changes, so every day is reindexed.
pub const INDEX_VERSION: i64 = 1;

/// The oldest SQLite with `contentless_delete`.
pub const MIN_SQLITE: (u32, u32) = (3, 43);

pub const SCHEMA: &str = r#"
    CREATE VIRTUAL TABLE IF NOT EXISTS history_search USING fts5(
        text,
        content = '',
        contentless_delete = 1,
        tokenize = 'unicode61 remove_diacritics 2'
    );
    -- rowid of history_search -> the entry it indexes.
    CREATE TABLE IF NOT EXISTS history_search_rows (
        row INTEGER PRIMARY KEY,
        entry_id TEXT NOT NULL UNIQUE,
        day TEXT NOT NULL,
        ts INTEGER NOT NULL
    );
    CREATE INDEX IF NOT EXISTS idx_history_search_rows_day ON history_search_rows(day);
    -- Each indexed day file as it was when indexed.
    CREATE TABLE IF NOT EXISTS history_search_days (
        day TEXT PRIMARY KEY,
        fingerprint TEXT NOT NULL,
        version INTEGER NOT NULL
    );
"#;

/// Where the shared index lives: the meetings `index.db`.
pub fn db_path(config: &crate::config::Config) -> PathBuf {
    let meetings = if config.meeting.storage_path == "auto" {
        crate::config::Config::data_dir().join("meetings")
    } else {
        PathBuf::from(&config.meeting.storage_path)
    };
    meetings.join("index.db")
}

#[derive(Debug, thiserror::Error)]
pub enum IndexError {
    #[error(
        "Nothing to search for. Give some words, for example: voxtype history search \"invoice\""
    )]
    EmptyQuery,
    #[error("History search index: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("History search index: {0}")]
    Io(#[from] std::io::Error),
    #[error("History files were updated, but another SQLite reader prevents clearing historical WAL pages. Close other history/meeting views and retry `voxtype history reindex`; index cleanup is not complete yet.")]
    WalReaders,
}

/// The file's identity as far as reindexing cares.
fn fingerprint(file: &DayFile) -> std::io::Result<String> {
    let meta = std::fs::metadata(&file.path)?;
    let mtime = meta
        .modified()?
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    Ok(format!(
        "{}:{}:{}",
        if file.gzipped { "gz" } else { "plain" },
        meta.len(),
        mtime
    ))
}

/// The text a row indexes.
fn indexed_text(entry: &HistoryEntry) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in [
        entry.text.as_str(),
        entry.corrected_text.as_deref().unwrap_or(""),
        entry.raw_text.as_str(),
    ] {
        if !part.trim().is_empty() && !parts.contains(&part) {
            parts.push(part);
        }
    }
    parts.join("\n")
}

/// One dictation that matched.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HistoryHit {
    pub id: String,
    /// When the dictation started, RFC 3339 with the local offset.
    pub date: String,
    /// The matching text with each match wrapped in « and ».
    pub snippet: String,
    pub snippet_plain: String,
    /// Where the matches are in `snippet_plain`, as [start, end) offsets in
    /// Unicode characters.
    pub highlights: Vec<[usize; 2]>,
    pub rank: f64,
    pub entry: HistoryEntry,
}

/// What to search for.
#[derive(Debug, Clone, Default)]
pub struct HistoryQuery {
    pub text: String,
    pub limit: u32,
    /// Only dictations on or after this day.
    pub since: Option<NaiveDate>,
}

/// The search index connection.
pub struct HistoryIndex {
    conn: Connection,
}

impl HistoryIndex {
    /// Open (and create) the history tables in the database at `path`.
    pub fn open(path: &Path) -> Result<Self, IndexError> {
        if let Some(parent) = path.parent() {
            super::store::ensure_private_dir(parent)?;
        }
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
            .open(path)?;
        if !file.metadata()?.is_file() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "History index must be a regular file",
            )
            .into());
        }
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        let conn = Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::default() | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )?;
        Self::with_connection(conn)
    }

    /// An in-memory index, for tests.
    pub fn open_in_memory() -> Result<Self, IndexError> {
        Self::with_connection(Connection::open_in_memory()?)
    }

    fn with_connection(conn: Connection) -> Result<Self, IndexError> {
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        // Must be on before anything is deleted through this connection.
        conn.execute_batch("PRAGMA secure_delete = ON;")?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn })
    }

    pub fn sqlite_version() -> (u32, u32) {
        let v = rusqlite::version_number() as u32;
        (v / 1_000_000, (v / 1_000) % 1_000)
    }

    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    fn remove_day_rows(&self, day: &str) -> rusqlite::Result<usize> {
        let mut stmt = self
            .conn
            .prepare_cached("SELECT row FROM history_search_rows WHERE day = ?1")?;
        let rows: Vec<i64> = stmt
            .query_map(params![day], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        for row in &rows {
            self.conn
                .execute("DELETE FROM history_search WHERE rowid = ?1", params![row])?;
        }
        self.conn.execute(
            "DELETE FROM history_search_rows WHERE day = ?1",
            params![day],
        )?;
        Ok(rows.len())
    }

    fn insert_entry(&self, day: &str, entry: &HistoryEntry) -> rusqlite::Result<()> {
        // Replace a row the entry already has, so a rewritten entry is
        // never indexed twice.
        if let Some(row) = self
            .conn
            .query_row(
                "SELECT row FROM history_search_rows WHERE entry_id = ?1",
                params![entry.id],
                |r| r.get::<_, i64>(0),
            )
            .optional()?
        {
            self.conn
                .execute("DELETE FROM history_search WHERE rowid = ?1", params![row])?;
            self.conn.execute(
                "DELETE FROM history_search_rows WHERE row = ?1",
                params![row],
            )?;
        }
        let text = indexed_text(entry);
        if text.is_empty() {
            return Ok(());
        }
        self.conn.execute(
            "INSERT INTO history_search_rows (entry_id, day, ts) VALUES (?1, ?2, ?3)",
            params![entry.id, day, entry.ts_start.timestamp()],
        )?;
        let row = self.conn.last_insert_rowid();
        self.conn.execute(
            "INSERT INTO history_search (rowid, text) VALUES (?1, ?2)",
            params![row, text],
        )?;
        Ok(())
    }

    fn mark_day(&self, day: &str, fingerprint: &str) -> rusqlite::Result<()> {
        self.conn
            .execute(
                "INSERT OR REPLACE INTO history_search_days (day, fingerprint, version) \
                 VALUES (?1, ?2, ?3)",
                params![day, fingerprint, INDEX_VERSION],
            )
            .map(|_| ())
    }

    /// Rewrite the term segments without deleted rows, so removed text
    /// leaves no trace in the index pages (which `secure_delete` then
    /// zeroes).
    fn purge(&self) -> rusqlite::Result<()> {
        self.conn
            .execute(
                "INSERT INTO history_search (history_search) VALUES ('optimize')",
                [],
            )
            .map(|_| ())
    }

    fn checkpoint(&self) -> Result<(), IndexError> {
        let busy: i64 = self
            .conn
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| row.get(0))?;
        if busy != 0 {
            return Err(IndexError::WalReaders);
        }
        Ok(())
    }

    /// Reconcile the authoritative files after an append. A single entry
    /// cannot certify a whole day's fingerprint: another append, correction
    /// or deletion may have missed its best-effort update.
    pub fn add(&self, store: &HistoryStore, _entry: &HistoryEntry) -> Result<(), IndexError> {
        self.sync(store).map(|_| ())
    }

    /// Bring the index up to date with the day files. Returns how many days
    /// were (re)indexed or dropped.
    pub fn sync(&self, store: &HistoryStore) -> Result<usize, IndexError> {
        let files = store.days()?;
        let mut indexed: HashMap<String, (String, i64)> = HashMap::new();
        {
            let mut stmt = self
                .conn
                .prepare("SELECT day, fingerprint, version FROM history_search_days")?;
            let rows =
                stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, (r.get(1)?, r.get(2)?))))?;
            for row in rows {
                let (day, state) = row?;
                indexed.insert(day, state);
            }
        }

        let mut changed = 0;
        let mut removed_any = false;
        let mut read_error = None;
        let tx = self.conn.unchecked_transaction()?;
        for file in &files {
            let day = file.date.to_string();
            let previous = indexed.remove(&day);
            let print = match fingerprint(file) {
                Ok(print) => print,
                Err(error) => {
                    removed_any |= self.remove_day_rows(&day)? > 0;
                    self.conn.execute(
                        "DELETE FROM history_search_days WHERE day = ?1",
                        params![day],
                    )?;
                    read_error.get_or_insert(error);
                    changed += 1;
                    continue;
                }
            };
            let current = previous.is_some_and(|(p, v)| p == print && v >= INDEX_VERSION);
            if current {
                continue;
            }
            removed_any |= self.remove_day_rows(&day)? > 0;
            let entries = match HistoryStore::read_day_file(file) {
                Ok(read) => read.entries,
                Err(error) => {
                    // Unreadable authority cannot certify old search terms.
                    // Commit their removal and missing-day cleanup even though
                    // the caller still receives this read failure.
                    self.conn.execute(
                        "DELETE FROM history_search_days WHERE day = ?1",
                        params![day],
                    )?;
                    read_error.get_or_insert(error);
                    changed += 1;
                    continue;
                }
            };
            for entry in entries {
                if entry.day() == Some(file.date) {
                    self.insert_entry(&day, &entry)?;
                }
            }
            self.mark_day(&day, &print)?;
            changed += 1;
        }
        // Days indexed whose file is gone: deleted, cleared or pruned.
        for day in indexed.into_keys() {
            removed_any |= self.remove_day_rows(&day)? > 0;
            self.conn.execute(
                "DELETE FROM history_search_days WHERE day = ?1",
                params![day],
            )?;
            changed += 1;
        }
        if removed_any {
            self.purge()?;
        }
        tx.commit()?;
        // Always retry: the previous sync may have committed its logical
        // deletion but reported a reader-blocked checkpoint afterwards.
        self.checkpoint()?;
        if let Some(error) = read_error {
            return Err(error.into());
        }
        Ok(changed)
    }

    /// Drop every history row and reindex from the files.
    pub fn rebuild(&self, store: &HistoryStore) -> Result<usize, IndexError> {
        self.conn.execute_batch(
            "DELETE FROM history_search; DELETE FROM history_search_rows; \
             DELETE FROM history_search_days;",
        )?;
        self.purge()?;
        self.sync(store)
    }

    /// Search dictations, newest best match first. Syncs first.
    pub fn search(
        &self,
        store: &HistoryStore,
        query: &HistoryQuery,
    ) -> Result<Vec<HistoryHit>, IndexError> {
        let fts = crate::meeting::search::fts_query(&query.text).ok_or(IndexError::EmptyQuery)?;
        self.sync(store)?;
        let since = query.since.map(|d| d.to_string());
        let mut stmt = self.conn.prepare(
            "SELECT r.entry_id, bm25(history_search) \
             FROM history_search s JOIN history_search_rows r ON r.row = s.rowid \
             WHERE history_search MATCH ?1 AND (?2 IS NULL OR r.day >= ?2) \
             ORDER BY bm25(history_search), r.ts DESC LIMIT ?3",
        )?;
        let ids: Vec<(String, f64)> = stmt
            .query_map(params![fts, since, query.limit.max(1)], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?
            .collect::<rusqlite::Result<_>>()?;

        let terms = query_terms(&query.text);
        let mut hits = Vec::new();
        for (id, rank) in ids {
            // The files are the record: an entry the index still names but
            // the file no longer has is skipped.
            let Some(entry) = store.find(&id)? else {
                continue;
            };
            let (snippet, snippet_plain, highlights) = snippet(entry.best_text(), &terms, 24);
            hits.push(HistoryHit {
                id,
                date: entry
                    .ts_start
                    .to_rfc3339_opts(chrono::SecondsFormat::Secs, false),
                snippet,
                snippet_plain,
                highlights,
                rank,
                entry,
            });
        }
        Ok(hits)
    }

    /// How many dictations are indexed.
    pub fn count(&self) -> Result<i64, IndexError> {
        Ok(self
            .conn
            .query_row("SELECT COUNT(*) FROM history_search_rows", [], |r| r.get(0))?)
    }
}

/// One result of a search across dictations and meetings.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "source", rename_all = "lowercase")]
pub enum AnyHit {
    Dictation(Box<HistoryHit>),
    Meeting(crate::meeting::search::SearchHit),
}

impl AnyHit {
    /// When the dictation or meeting started, as a Unix timestamp.
    pub fn timestamp(&self) -> i64 {
        match self {
            AnyHit::Dictation(h) => h.entry.ts_start.timestamp(),
            AnyHit::Meeting(h) => chrono::DateTime::parse_from_rfc3339(&h.date)
                .map(|d| d.timestamp())
                .unwrap_or(0),
        }
    }
}

/// One search over dictations and meetings, newest first. Both indexes live
/// in the same `index.db`; each keeps its own relevance order before the
/// two are merged by date, since their ranks are not comparable.
pub fn search_everything(
    index: &HistoryIndex,
    store: &HistoryStore,
    meetings: Option<&crate::meeting::MeetingStorage>,
    query: &HistoryQuery,
) -> Result<Vec<AnyHit>, IndexError> {
    let mut hits: Vec<AnyHit> = index
        .search(store, query)?
        .into_iter()
        .map(|h| AnyHit::Dictation(Box::new(h)))
        .collect();
    if let Some(meetings) = meetings {
        let since = query
            .since
            .and_then(|d| crate::meeting::search::parse_since(&d.format("%Y-%m-%d").to_string()));
        let found = meetings
            .search(&crate::meeting::search::SearchQuery {
                text: query.text.clone(),
                limit: query.limit,
                speaker: None,
                since,
            })
            .map_err(|e| IndexError::Io(std::io::Error::other(e.to_string())))?;
        hits.extend(found.into_iter().map(AnyHit::Meeting));
    }
    hits.sort_by_key(|h| std::cmp::Reverse(h.timestamp()));
    hits.truncate(query.limit.max(1) as usize);
    Ok(hits)
}

/// The words of a query, lowercased, with a trailing `*` kept as a prefix
/// marker. Quotes only group; each word is highlighted on its own.
fn query_terms(input: &str) -> Vec<(String, bool)> {
    input
        .split(|c: char| c.is_whitespace() || c == '"')
        .filter(|w| !w.is_empty())
        .filter_map(|w| {
            let (w, prefix) = match w.strip_suffix('*') {
                Some(stem) => (stem, true),
                None => (w, false),
            };
            let w: String = w
                .chars()
                .filter(|c| c.is_alphanumeric())
                .flat_map(char::to_lowercase)
                .collect();
            (!w.is_empty()).then_some((w, prefix))
        })
        .collect()
}

fn word_matches(word: &str, terms: &[(String, bool)]) -> bool {
    let folded: String = word
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect();
    !folded.is_empty()
        && terms.iter().any(|(t, prefix)| {
            if *prefix {
                folded.starts_with(t.as_str())
            } else {
                folded == *t
            }
        })
}

/// Up to `max_words` words of `text` around the first match, matches marked
/// with « », plus the same text unmarked and the match offsets.
pub fn snippet(
    text: &str,
    terms: &[(String, bool)],
    max_words: usize,
) -> (String, String, Vec<[usize; 2]>) {
    let words: Vec<&str> = text.split_whitespace().collect();
    let first = words
        .iter()
        .position(|w| word_matches(w, terms))
        .unwrap_or(0);
    let start = first.saturating_sub(max_words / 3);
    let end = (start + max_words).min(words.len());
    let start = end.saturating_sub(max_words).min(start);

    let mut marked = String::new();
    let mut plain = String::new();
    let mut highlights = Vec::new();
    let mut chars = 0usize;
    fn push(s: &str, marked: &mut String, plain: &mut String, chars: &mut usize) {
        marked.push_str(s);
        plain.push_str(s);
        *chars += s.chars().count();
    }
    if start > 0 {
        push("…", &mut marked, &mut plain, &mut chars);
    }
    for (i, word) in words[start..end].iter().enumerate() {
        if i > 0 {
            push(" ", &mut marked, &mut plain, &mut chars);
        }
        if word_matches(word, terms) {
            marked.push('«');
            let begin = chars;
            push(word, &mut marked, &mut plain, &mut chars);
            marked.push('»');
            highlights.push([begin, chars]);
        } else {
            push(word, &mut marked, &mut plain, &mut chars);
        }
    }
    if end < words.len() {
        push("…", &mut marked, &mut plain, &mut chars);
    }
    (marked, plain, highlights)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::entry::Outcome;
    use chrono::{Local, TimeZone};

    fn setup() -> (tempfile::TempDir, HistoryStore, PathBuf) {
        let t = tempfile::tempdir().unwrap();
        let store = HistoryStore::open(t.path().join("history")).unwrap();
        let db = t.path().join("meetings").join("index.db");
        (t, store, db)
    }

    fn entry(d: u32, h: u32, text: &str) -> HistoryEntry {
        let mut e = HistoryEntry::new(Local.with_ymd_and_hms(2026, 11, d, h, 0, 0).unwrap());
        e.raw_text = text.to_lowercase();
        e.text = text.to_string();
        e.set_outcome(Outcome::Delivered, None);
        e
    }

    fn q(text: &str) -> HistoryQuery {
        HistoryQuery {
            text: text.into(),
            limit: 20,
            since: None,
        }
    }

    fn ids(hits: &[HistoryHit]) -> Vec<&str> {
        hits.iter().map(|h| h.id.as_str()).collect()
    }

    #[test]
    fn the_bundled_sqlite_supports_contentless_delete() {
        assert!(
            HistoryIndex::sqlite_version() >= MIN_SQLITE,
            "SQLite {:?} is older than {:?}",
            HistoryIndex::sqlite_version(),
            MIN_SQLITE
        );
    }

    #[test]
    fn finds_dictations_and_reads_their_text_from_the_files() {
        let (_t, store, db) = setup();
        let a = entry(13, 9, "Send the invoice to Dana today.");
        let b = entry(14, 9, "Lunch at noon?");
        store.append(&a).unwrap();
        store.append(&b).unwrap();
        let index = HistoryIndex::open(&db).unwrap();

        let hits = index.search(&store, &q("invoice")).unwrap();
        assert_eq!(ids(&hits), vec![a.id.as_str()]);
        assert_eq!(hits[0].snippet, "Send the «invoice» to Dana today.");
        assert_eq!(hits[0].highlights, vec![[9, 16]]);
        assert_eq!(hits[0].entry, a);

        // Prefix and diacritic-insensitive matching come from FTS5.
        assert_eq!(
            ids(&index.search(&store, &q("inv*")).unwrap()),
            vec![a.id.as_str()]
        );
        assert!(index.search(&store, &q("nothing")).unwrap().is_empty());
        assert!(matches!(
            index.search(&store, &q("  ")),
            Err(IndexError::EmptyQuery)
        ));
        let since = HistoryQuery {
            since: NaiveDate::from_ymd_opt(2026, 11, 14),
            ..q("invoice")
        };
        assert!(index.search(&store, &since).unwrap().is_empty());
    }

    #[test]
    fn the_index_keeps_no_copy_of_the_text() {
        let (_t, store, db) = setup();
        store
            .append(&entry(14, 9, "Zanzibarquux appears once."))
            .unwrap();
        let index = HistoryIndex::open(&db).unwrap();
        index.sync(&store).unwrap();
        // Contentless: the column reads back as NULL.
        let text: Option<String> = index
            .connection()
            .query_row("SELECT text FROM history_search LIMIT 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(text, None);
    }

    #[test]
    fn follows_appends_corrections_deletes_and_pruning() {
        let (_t, store, db) = setup();
        let index = HistoryIndex::open(&db).unwrap();
        let a = entry(14, 9, "draft the proposal");
        store.append(&a).unwrap();
        assert_eq!(index.search(&store, &q("proposal")).unwrap().len(), 1);

        // The daemon's path: append, then add.
        let b = entry(14, 10, "proposal follow up");
        store.append(&b).unwrap();
        index.add(&store, &b).unwrap();
        assert_eq!(index.sync(&store).unwrap(), 0, "add left nothing for sync");
        assert_eq!(index.search(&store, &q("proposal")).unwrap().len(), 2);

        store
            .update(&a.id, |e| {
                e.corrected_text = Some("Draft the quotation.".into())
            })
            .unwrap();
        assert_eq!(
            ids(&index.search(&store, &q("quotation")).unwrap()),
            vec![a.id.as_str()]
        );
        assert_eq!(index.count().unwrap(), 2);

        store.delete(&b.id).unwrap();
        assert_eq!(
            ids(&index.search(&store, &q("proposal")).unwrap()),
            vec![a.id.as_str()]
        );

        store.clear(None).unwrap();
        assert!(index.search(&store, &q("proposal")).unwrap().is_empty());
        assert_eq!(index.count().unwrap(), 0);
    }

    #[test]
    fn rebuilds_from_plain_and_gzipped_files() {
        let (_t, store, db) = setup();
        store.append(&entry(1, 9, "ancient gzipped words")).unwrap();
        store.append(&entry(14, 9, "fresh plain words")).unwrap();
        store
            .compress_old(NaiveDate::from_ymd_opt(2026, 11, 14).unwrap())
            .unwrap();
        let index = HistoryIndex::open(&db).unwrap();
        assert_eq!(index.rebuild(&store).unwrap(), 2);
        assert_eq!(index.search(&store, &q("words")).unwrap().len(), 2);
        assert_eq!(index.search(&store, &q("gzipped")).unwrap().len(), 1);

        // Losing the database loses nothing.
        drop(index);
        std::fs::remove_file(&db).unwrap();
        let index = HistoryIndex::open(&db).unwrap();
        assert_eq!(index.search(&store, &q("words")).unwrap().len(), 2);
    }

    #[test]
    fn lives_beside_the_meeting_tables() {
        let (t, store, db) = setup();
        let meetings = crate::meeting::MeetingStorage::open(crate::meeting::StorageConfig {
            storage_path: t.path().join("meetings"),
            ..Default::default()
        })
        .unwrap();
        store.append(&entry(14, 9, "shared database")).unwrap();
        let index = HistoryIndex::open(&db).unwrap();
        assert_eq!(index.search(&store, &q("shared")).unwrap().len(), 1);
        drop(meetings);
        let conn = Connection::open(&db).unwrap();
        for table in ["meetings", "meeting_search", "history_search"] {
            let n: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE name = ?1",
                    params![table],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(n, 1, "{table}");
        }
    }

    #[test]
    fn one_search_covers_dictations_and_meetings() {
        use crate::meeting::{MeetingMetadata, MeetingStorage, StorageConfig};
        let (t, store, db) = setup();
        let meetings = MeetingStorage::open(StorageConfig {
            storage_path: t.path().join("meetings"),
            ..Default::default()
        })
        .unwrap();
        let mut meta = MeetingMetadata::new(Some("Budget review".into()));
        meta.started_at = chrono::Utc::now() - chrono::Duration::days(400);
        meetings.create_meeting(&meta).unwrap();
        meetings.backfill_search_index().unwrap();

        store
            .append(&entry(14, 9, "check the budget numbers"))
            .unwrap();
        let index = HistoryIndex::open(&db).unwrap();
        let hits = search_everything(&index, &store, Some(&meetings), &q("budget")).unwrap();
        assert_eq!(hits.len(), 2);
        assert!(matches!(hits[0], AnyHit::Dictation(_)), "newest first");
        assert!(matches!(hits[1], AnyHit::Meeting(_)));
        let json = serde_json::to_value(&hits).unwrap();
        assert_eq!(json[0]["source"], "dictation");
        assert_eq!(json[1]["source"], "meeting");

        let only = search_everything(&index, &store, None, &q("budget")).unwrap();
        assert_eq!(only.len(), 1);
    }

    /// Every file SQLite keeps for the database, read as raw bytes.
    fn db_bytes(db: &Path) -> Vec<u8> {
        let mut bytes = Vec::new();
        for suffix in ["", "-journal", "-wal"] {
            let mut path = db.as_os_str().to_owned();
            path.push(suffix);
            if let Ok(b) = std::fs::read(PathBuf::from(path)) {
                bytes.extend(b);
            }
        }
        bytes
    }

    fn contains(haystack: &[u8], needle: &str) -> bool {
        haystack
            .windows(needle.len())
            .any(|w| w == needle.as_bytes())
    }

    #[test]
    fn a_deleted_dictation_leaves_no_text_in_the_database_bytes() {
        let (_t, store, db) = setup();
        let index = HistoryIndex::open(&db).unwrap();
        // Enough neighbours that the secret shares pages with live rows.
        for h in 0..20 {
            let e = entry(14, h, &format!("ordinary filler dictation number {h}"));
            store.append(&e).unwrap();
        }
        let secret = entry(14, 21, "my password is xylophonegardenia");
        store.append(&secret).unwrap();
        index.sync(&store).unwrap();
        assert!(
            contains(&db_bytes(&db), "xylophonegardenia"),
            "the term is in the index before the delete"
        );

        store.delete(&secret.id).unwrap();
        index.sync(&store).unwrap();
        assert!(index
            .search(&store, &q("xylophonegardenia"))
            .unwrap()
            .is_empty());
        drop(index);
        assert!(
            !contains(&db_bytes(&db), "xylophonegardenia"),
            "deleted text must not linger in the database file"
        );
    }

    #[test]
    fn snippets_window_long_text_around_the_first_match() {
        let terms = query_terms("needle");
        let text = (0..40)
            .map(|i| {
                if i == 30 {
                    "needle".to_string()
                } else {
                    format!("w{i}")
                }
            })
            .collect::<Vec<_>>()
            .join(" ");
        let (marked, plain, hl) = snippet(&text, &terms, 10);
        assert!(marked.starts_with('…') && marked.ends_with('…'), "{marked}");
        assert!(marked.contains("«needle»"));
        let chars: Vec<char> = plain.chars().collect();
        let [a, b] = hl[0];
        assert_eq!(chars[a..b].iter().collect::<String>(), "needle");
        assert_eq!(
            query_terms("\"Café au\" lait*"),
            vec![
                ("café".into(), false),
                ("au".into(), false),
                ("lait".into(), true)
            ]
        );
    }
}
