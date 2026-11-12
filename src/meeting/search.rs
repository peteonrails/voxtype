//! Full-text search across meetings: `voxtype meeting search`.
//!
//! An SQLite FTS5 table in index.db holds one row per transcript line as it
//! is displayed (labels, reassignments and text edits applied, deleted
//! lines left out), plus each meeting's title, notes and summary. The
//! storage writers keep it current: `save_transcript` for lines and the
//! title, `set_notes` for notes, `save_summary` for the summary. Meetings
//! recorded before the index existed are indexed on the first search.
//!
//! `[meeting] my_name` is applied when results are read, never stored, so
//! changing it needs no reindex.

use chrono::{DateTime, TimeZone, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use super::data::{MeetingSummary, Transcript, MY_SPEAKER_KEY};

/// Bumped when what is indexed changes, so old meetings are reindexed.
pub const INDEX_VERSION: i64 = 1;

pub const SCHEMA: &str = r#"
    CREATE VIRTUAL TABLE IF NOT EXISTS meeting_search USING fts5(
        text,
        meeting_id UNINDEXED,
        kind UNINDEXED,
        segment_id UNINDEXED,
        start_ms UNINDEXED,
        speaker UNINDEXED,
        speaker_key UNINDEXED,
        tokenize = 'unicode61 remove_diacritics 2'
    );
    CREATE TABLE IF NOT EXISTS meeting_search_state (
        meeting_id TEXT PRIMARY KEY,
        version INTEGER NOT NULL
    );
"#;

/// What a row is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum HitKind {
    Segment,
    Title,
    Notes,
    Summary,
}

impl HitKind {
    fn as_str(self) -> &'static str {
        match self {
            HitKind::Segment => "segment",
            HitKind::Title => "title",
            HitKind::Notes => "notes",
            HitKind::Summary => "summary",
        }
    }

    fn parse(s: &str) -> Self {
        match s {
            "title" => HitKind::Title,
            "notes" => HitKind::Notes,
            "summary" => HitKind::Summary,
            _ => HitKind::Segment,
        }
    }
}

fn clear(conn: &Connection, meeting_id: &str, kind: Option<HitKind>) -> rusqlite::Result<()> {
    match kind {
        Some(kind) => conn.execute(
            "DELETE FROM meeting_search WHERE meeting_id = ?1 AND kind = ?2",
            params![meeting_id, kind.as_str()],
        ),
        None => conn.execute(
            "DELETE FROM meeting_search WHERE meeting_id = ?1",
            params![meeting_id],
        ),
    }
    .map(|_| ())
}

fn insert_text(
    conn: &Connection,
    meeting_id: &str,
    kind: HitKind,
    text: &str,
) -> rusqlite::Result<()> {
    if text.trim().is_empty() {
        return Ok(());
    }
    conn.execute(
        "INSERT INTO meeting_search (text, meeting_id, kind) VALUES (?1, ?2, ?3)",
        params![text, meeting_id, kind.as_str()],
    )
    .map(|_| ())
}

/// Replace a meeting's lines. `transcript` is as displayed: edits applied,
/// deleted lines removed, and without `my_name`.
pub fn index_lines(
    conn: &Connection,
    meeting_id: &str,
    transcript: &Transcript,
) -> rusqlite::Result<()> {
    clear(conn, meeting_id, Some(HitKind::Segment))?;
    let mut stmt = conn.prepare_cached(
        "INSERT INTO meeting_search \
         (text, meeting_id, kind, segment_id, start_ms, speaker, speaker_key) \
         VALUES (?1, ?2, 'segment', ?3, ?4, ?5, ?6)",
    )?;
    for s in &transcript.segments {
        if s.deleted || s.text.trim().is_empty() {
            continue;
        }
        stmt.execute(params![
            s.text,
            meeting_id,
            s.id,
            s.start_ms as i64,
            s.speaker_display(),
            s.speaker_key(),
        ])?;
    }
    Ok(())
}

pub fn index_title(
    conn: &Connection,
    meeting_id: &str,
    title: Option<&str>,
) -> rusqlite::Result<()> {
    clear(conn, meeting_id, Some(HitKind::Title))?;
    insert_text(conn, meeting_id, HitKind::Title, title.unwrap_or(""))
}

pub fn index_notes(conn: &Connection, meeting_id: &str, notes: &str) -> rusqlite::Result<()> {
    clear(conn, meeting_id, Some(HitKind::Notes))?;
    insert_text(conn, meeting_id, HitKind::Notes, notes)
}

/// The summary's text: overview, key points, decisions, action items and
/// chapter titles, one per line.
pub fn summary_text(summary: &MeetingSummary) -> String {
    let mut parts = vec![summary.summary.clone()];
    parts.extend(summary.key_points.iter().cloned());
    parts.extend(summary.decisions.iter().cloned());
    parts.extend(summary.action_items.iter().map(|a| match &a.assignee {
        Some(who) => format!("{} ({})", a.description, who),
        None => a.description.clone(),
    }));
    parts.extend(summary.chapters.iter().map(|c| c.title.clone()));
    parts.retain(|p| !p.trim().is_empty());
    parts.join("\n")
}

pub fn index_summary(
    conn: &Connection,
    meeting_id: &str,
    summary: Option<&MeetingSummary>,
) -> rusqlite::Result<()> {
    clear(conn, meeting_id, Some(HitKind::Summary))?;
    match summary {
        Some(s) => insert_text(conn, meeting_id, HitKind::Summary, &summary_text(s)),
        None => Ok(()),
    }
}

pub fn forget(conn: &Connection, meeting_id: &str) -> rusqlite::Result<()> {
    clear(conn, meeting_id, None)?;
    conn.execute(
        "DELETE FROM meeting_search_state WHERE meeting_id = ?1",
        params![meeting_id],
    )
    .map(|_| ())
}

pub fn mark_indexed(conn: &Connection, meeting_id: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO meeting_search_state (meeting_id, version) VALUES (?1, ?2)",
        params![meeting_id, INDEX_VERSION],
    )
    .map(|_| ())
}

pub fn is_indexed(conn: &Connection, meeting_id: &str) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT version FROM meeting_search_state WHERE meeting_id = ?1",
        params![meeting_id],
        |row| row.get::<_, i64>(0),
    )
    .optional()
    .map(|v| v.is_some_and(|v| v >= INDEX_VERSION))
}

/// Meetings whose index is missing or out of date.
pub fn unindexed_meetings(conn: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT m.id FROM meetings m LEFT JOIN meeting_search_state s ON s.meeting_id = m.id \
         WHERE s.version IS NULL OR s.version < ?1",
    )?;
    let ids = stmt
        .query_map(params![INDEX_VERSION], |row| row.get(0))?
        .collect();
    ids
}

/// Turn what the user typed into an FTS5 query: every word must appear,
/// "quoted phrases" stay together, and a trailing `*` makes a prefix.
/// Everything is quoted, so FTS5 operators in the input are just words.
pub fn fts_query(input: &str) -> Option<String> {
    let mut terms = Vec::new();
    let mut rest = input.trim();
    while !rest.is_empty() {
        let (term, tail) = if let Some(after) = rest.strip_prefix('"') {
            match after.find('"') {
                Some(end) => (&after[..end], &after[end + 1..]),
                None => (after, ""),
            }
        } else {
            match rest.find(char::is_whitespace) {
                Some(end) => (&rest[..end], &rest[end..]),
                None => (rest, ""),
            }
        };
        let (word, prefix) = match term.strip_suffix('*') {
            Some(w) if !w.trim().is_empty() => (w, true),
            _ => (term, false),
        };
        let word = word.trim();
        if !word.is_empty() {
            let quoted = format!("\"{}\"", word.replace('"', "\"\""));
            terms.push(if prefix { format!("{quoted}*") } else { quoted });
        }
        rest = tail.trim_start();
    }
    (!terms.is_empty()).then(|| terms.join(" "))
}

/// What to search for.
#[derive(Debug, Clone, Default)]
pub struct SearchQuery {
    pub text: String,
    pub limit: u32,
    /// Only lines spoken by this speaker (case-insensitive), as displayed.
    pub speaker: Option<String>,
    /// Only meetings started at or after this time.
    pub since: Option<DateTime<Utc>>,
}

/// One match.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SearchHit {
    pub meeting_id: String,
    pub title: String,
    /// When the meeting started, RFC 3339 in UTC.
    pub date: String,
    pub kind: HitKind,
    /// The line, for `kind = "segment"`; otherwise null.
    pub segment_id: Option<u32>,
    pub start_ms: Option<u64>,
    pub speaker: Option<String>,
    /// The matching text around the match, each match wrapped in « and ».
    pub snippet: String,
    /// The same text without the markers.
    pub snippet_plain: String,
    /// Where the matches are in `snippet_plain`, as [start, end) offsets in
    /// Unicode characters.
    pub highlights: Vec<[usize; 2]>,
    /// FTS5 bm25 rank; lower is better.
    pub rank: f64,
}

const MARK_START: char = '\u{1}';
const MARK_END: char = '\u{2}';

/// Split SQLite's marked snippet into the « » form, the plain text and the
/// match offsets.
fn mark(raw: &str) -> (String, String, Vec<[usize; 2]>) {
    let mut marked = String::new();
    let mut plain = String::new();
    let mut highlights = Vec::new();
    let mut chars = 0usize;
    let mut open = None;
    for c in raw.chars() {
        match c {
            MARK_START => {
                marked.push('«');
                open = Some(chars);
            }
            MARK_END => {
                marked.push('»');
                if let Some(start) = open.take() {
                    highlights.push([start, chars]);
                }
            }
            c => {
                marked.push(c);
                plain.push(c);
                chars += 1;
            }
        }
    }
    (marked, plain, highlights)
}

fn display_title(title: Option<String>, started_at: DateTime<Utc>) -> String {
    title
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| started_at.format("Meeting %Y-%m-%d %H:%M").to_string())
}

/// Run a search. The caller brings the index up to date first.
pub fn search(
    conn: &Connection,
    query: &SearchQuery,
    my_name: Option<&str>,
) -> Result<Vec<SearchHit>, SearchError> {
    let fts = fts_query(&query.text).ok_or(SearchError::EmptyQuery)?;
    let speaker = query
        .speaker
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    // Asking for my_name also finds the "You" lines it is shown on.
    let speaker_is_me = speaker.is_some_and(|s| my_name.is_some_and(|n| n.eq_ignore_ascii_case(s)));
    let mut stmt = conn.prepare(
        "SELECT s.meeting_id, s.kind, s.segment_id, s.start_ms, s.speaker, s.speaker_key, \
                snippet(meeting_search, 0, char(1), char(2), '…', 16), \
                bm25(meeting_search), m.title, m.started_at \
         FROM meeting_search s JOIN meetings m ON m.id = s.meeting_id \
         WHERE meeting_search MATCH ?1 \
           AND (?2 IS NULL OR m.started_at >= ?2) \
           AND (?3 IS NULL OR (s.kind = 'segment' AND (s.speaker = ?3 COLLATE NOCASE \
                OR (?4 AND s.speaker_key = ?5 AND s.speaker = ?5)))) \
         ORDER BY bm25(meeting_search), m.started_at DESC, s.start_ms \
         LIMIT ?6",
    )?;
    let rows = stmt.query_map(
        params![
            fts,
            query.since.map(|t| t.timestamp()),
            speaker,
            speaker_is_me,
            MY_SPEAKER_KEY,
            query.limit.max(1),
        ],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<i64>>(2)?,
                row.get::<_, Option<i64>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, f64>(7)?,
                row.get::<_, Option<String>>(8)?,
                row.get::<_, i64>(9)?,
            ))
        },
    )?;
    let mut hits = Vec::new();
    for row in rows {
        let (id, kind, seg, start, speaker, key, raw, rank, title, started) = row?;
        let started_at = Utc.timestamp_opt(started, 0).single().unwrap_or_default();
        let speaker = match (speaker, key) {
            (Some(shown), Some(key)) if shown == MY_SPEAKER_KEY && key == MY_SPEAKER_KEY => {
                Some(my_name.map(str::to_string).unwrap_or(shown))
            }
            (shown, _) => shown,
        };
        let (snippet, snippet_plain, highlights) = mark(&raw);
        hits.push(SearchHit {
            meeting_id: id,
            title: display_title(title, started_at),
            date: started_at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            kind: HitKind::parse(&kind),
            segment_id: seg.map(|s| s as u32),
            start_ms: start.map(|s| s as u64),
            speaker,
            snippet,
            snippet_plain,
            highlights,
            rank,
        });
    }
    Ok(hits)
}

#[derive(Debug, thiserror::Error)]
pub enum SearchError {
    #[error(
        "Nothing to search for. Give some words, for example: voxtype meeting search \"budget\""
    )]
    EmptyQuery,
    #[error("Search failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

/// Read `--since`: a date (`2026-11-01`, local midnight) or an RFC 3339 time.
pub fn parse_since(s: &str) -> Option<DateTime<Utc>> {
    let s = s.trim();
    if let Ok(t) = DateTime::parse_from_rfc3339(s) {
        return Some(t.with_timezone(&Utc));
    }
    let day = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()?;
    chrono::Local
        .from_local_datetime(&day.and_hms_opt(0, 0, 0)?)
        .earliest()
        .map(|t| t.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queries_are_quoted_so_operators_are_words() {
        assert_eq!(fts_query("budget review").unwrap(), r#""budget" "review""#);
        assert_eq!(
            fts_query(r#""next quarter" plan*"#).unwrap(),
            r#""next quarter" "plan"*"#
        );
        assert_eq!(fts_query("NOT OR AND").unwrap(), r#""NOT" "OR" "AND""#);
        assert_eq!(fts_query(r#"say "hi"#).unwrap(), r#""say" "hi""#);
        assert_eq!(fts_query(r#"a"b"#).unwrap(), r#""a""b""#);
        assert_eq!(fts_query("   "), None);
        assert_eq!(fts_query("*"), Some(r#""*""#.to_string()));
    }

    #[test]
    fn marks_become_guillemets_and_offsets() {
        let raw =
            format!("…we should {MARK_START}ship{MARK_END} it, café {MARK_START}now{MARK_END}");
        let (marked, plain, hl) = mark(&raw);
        assert_eq!(marked, "…we should «ship» it, café «now»");
        assert_eq!(plain, "…we should ship it, café now");
        let chars: Vec<char> = plain.chars().collect();
        let words: Vec<String> = hl
            .iter()
            .map(|[a, b]| chars[*a..*b].iter().collect())
            .collect();
        assert_eq!(words, vec!["ship", "now"]);
    }

    #[test]
    fn since_takes_a_date_or_a_time() {
        assert!(parse_since("2026-11-01").is_some());
        assert_eq!(
            parse_since("2026-11-01T10:00:00Z").unwrap().to_rfc3339(),
            "2026-11-01T10:00:00+00:00"
        );
        assert_eq!(parse_since("last tuesday"), None);
    }

    #[test]
    fn summary_text_covers_every_part() {
        let summary = MeetingSummary {
            summary: "Overview".into(),
            key_points: vec!["Point".into()],
            action_items: vec![super::super::data::ActionItem {
                description: "Send deck".into(),
                assignee: Some("Dana".into()),
                due_date: None,
                completed: false,
            }],
            decisions: vec!["Ship Friday".into()],
            chapters: vec![super::super::data::Chapter {
                start_ms: 0,
                title: "Intro".into(),
            }],
            generated_at: Utc::now(),
            model: None,
        };
        assert_eq!(
            summary_text(&summary),
            "Overview\nPoint\nShip Friday\nSend deck (Dana)\nIntro"
        );
    }
}
