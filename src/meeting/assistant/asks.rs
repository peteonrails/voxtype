//! `asks.jsonl` in the meeting folder: every question and its answer.

use super::prompt::Citation;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::Path;

pub const ASKS_FILE: &str = "asks.jsonl";

/// One question and its answer, one JSON object per line.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AskRecord {
    /// Unique per ask; the notification Open action names it.
    pub id: String,
    pub time: DateTime<Utc>,
    pub question: String,
    pub answer: String,
    /// `claude`, `codex`, or `local:<model>`.
    pub agent: String,
    /// What the UI shows: "Claude", "Local (llama3.2)".
    pub agent_name: String,
    #[serde(default)]
    pub citations: Vec<Citation>,
    /// `--last`, in seconds, when the question was about a window.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_secs: Option<u64>,
}

/// Append `record` as one line. A single write with O_APPEND, so two asks
/// finishing together do not interleave.
pub fn append(folder: &Path, record: &AskRecord) -> std::io::Result<()> {
    let mut line = serde_json::to_string(record).map_err(std::io::Error::other)?;
    line.push('\n');
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(folder.join(ASKS_FILE))?;
    f.write_all(line.as_bytes())
}

/// Every record in the folder, oldest first. Unreadable lines are skipped.
pub fn load(folder: &Path) -> Vec<AskRecord> {
    std::fs::read_to_string(folder.join(ASKS_FILE))
        .map(|text| {
            text.lines()
                .filter_map(|l| serde_json::from_str(l).ok())
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(q: &str) -> AskRecord {
        AskRecord {
            id: uuid::Uuid::new_v4().to_string(),
            time: Utc::now(),
            question: q.into(),
            answer: "a [00:41]".into(),
            agent: "claude".into(),
            agent_name: "Claude".into(),
            citations: super::super::prompt::citations("a [00:41]"),
            last_secs: Some(300),
        }
    }

    #[test]
    fn records_append_and_load_in_order() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load(dir.path()).is_empty());
        append(dir.path(), &record("one")).unwrap();
        append(dir.path(), &record("two")).unwrap();
        std::fs::OpenOptions::new()
            .append(true)
            .open(dir.path().join(ASKS_FILE))
            .unwrap()
            .write_all(b"garbage\n")
            .unwrap();
        let got = load(dir.path());
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].question, "one");
        assert_eq!(got[1].citations[0].ms, 41_000);
    }
}
