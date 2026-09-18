//! `voxtype stats` — dictation stats parsed from the journal.
//!
//! The daemon appends one `INFO DictationStats: ...` line per dictation
//! (see `DictationTiming` in `daemon.rs`). This command reads the same
//! user journal that DictDrawer archives from, so no extra persistence
//! format is introduced; journald is the store.

use anyhow::{anyhow, Context};
use serde::Serialize;
use voxtype::config;

/// Journal lines scanned. Matches DictDrawer's archive import limit
/// (JOURNAL_LIMIT), so both see the same window of history.
const JOURNAL_LIMIT: usize = 50000;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) struct DictationStat {
    /// Journal record timestamp, in microseconds.
    ts_micros: u64,
    words: usize,
    audio_secs: f32,
    /// Speech-only duration from VAD, when it ran.
    speech_secs: Option<f32>,
    /// WPM: words over speech time (or audio time without VAD), when
    /// the recording was long enough to be meaningful.
    wpm: Option<f32>,
}

struct Parsed<'a> {
    words: usize,
    audio_secs: f32,
    speech_secs: Option<&'a str>,
    wpm: Option<&'a str>,
}

/// Parse the log payload after `INFO DictationStats:`, produced by the
/// daemon's format strings. `speech`/`wpm` are `none` when unavailable.
fn parse_stats_message(message: &str) -> Option<Parsed<'_>> {
    let rest = message.split_once("DictationStats: ")?.1;
    if !rest.starts_with("words=") {
        return None;
    }
    let mut p = Parsed {
        words: 0,
        audio_secs: 0.0,
        speech_secs: None,
        wpm: None,
    };
    for token in rest.split_whitespace() {
        let (key, value) = token.split_once('=')?;
        match key {
            "words" => p.words = value.parse().ok()?,
            "audio" => p.audio_secs = value.trim_end_matches('s').parse().ok()?,
            "speech" => match value.trim_end_matches('s') {
                "none" => p.speech_secs = None,
                secs => p.speech_secs = Some(secs),
            },
            "wpm" => match value {
                "none" => p.wpm = None,
                w => p.wpm = Some(w),
            },
            _ => return None,
        }
    }
    Some(p)
}

/// Journal entries as (timestamp, message), plus a warning when the journal
/// itself was unavailable.
type JournalRead = (Vec<(u64, String)>, Option<String>);

/// Read the user journal for the voxtype daemon and yield entries newest
/// first, plus a message when the journal itself was unavailable.
fn read_journal() -> Result<JournalRead, anyhow::Error> {
    let output = std::process::Command::new("journalctl")
        .args([
            "--user",
            "-u",
            "voxtype.service",
            "-o",
            "json",
            "--no-pager",
            "-r",
            "-n",
            &JOURNAL_LIMIT.to_string(),
            // Filter at the journald layer: a full 50k-record pull is tens of
            // MB and dominated by journalctl itself; grep shrinks it ~1000x
            // (seconds to a fraction of a second).
            "--grep=DictationStats",
        ])
        .output()
        .map_err(|e| anyhow!("Cannot read the journal: {e}. Check that journalctl is available."))
        .with_context(|| "reading voxtype.service journal")?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut entries = Vec::new();
    for line in stdout.lines() {
        let Ok(record) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let Some(message) = decode_message(&record["MESSAGE"]) else {
            continue;
        };
        let Some(ts) = record["__REALTIME_TIMESTAMP"].as_u64().or_else(|| {
            record["__REALTIME_TIMESTAMP"]
                .as_str()
                .and_then(|s| s.parse().ok())
        }) else {
            continue;
        };
        // Keep the whole message; marker matching happens per line below.
        entries.push((ts, message.to_string()));
    }
    let warning = if output.status.success() {
        None
    } else {
        Some(format!(
            "journalctl exited with {}; history may be incomplete.",
            output.status
        ))
    };
    Ok((entries, warning))
}

/// journald JSON can carry MESSAGE as a string or as an array of bytes
/// (when the writer emitted bytes); decode both like DictDrawer does.
fn decode_message(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Array(items) => {
            let bytes: Result<Vec<u8>, _> = items
                .iter()
                .map(|i| i.as_u64().map(|v| v as u8).ok_or(()))
                .collect();
            bytes.ok().map(|b| String::from_utf8_lossy(&b).into_owned())
        }
        _ => None,
    }
}

/// Extract [`DictationStat`] values from journal messages, newest first
/// (journal order).
fn collect_stats(messages: &[(u64, String)]) -> Vec<DictationStat> {
    let mut stats = Vec::new();
    for (ts, message) in messages {
        for line in message.lines() {
            let clean = strip_ansi(line);
            if let Some(idx) = clean.find("DictationStats: ") {
                // Dictated text is echoed verbatim (Debug-escaped, so it stays
                // on one log line) inside `Transcribed:` and various
                // post-processing lines, so a fake stat payload can appear
                // mid-line in the journal. The real stats line is emitted on
                // its own with a bare timestamp + `INFO` prefix; reject any
                // marker whose prefix isn't that shape.
                if !marker_has_real_prefix(&clean[..idx]) {
                    continue;
                }
                if let Some(p) = parse_stats_message(&clean[idx..]) {
                    stats.push(DictationStat {
                        ts_micros: *ts,
                        words: p.words,
                        audio_secs: p.audio_secs,
                        speech_secs: p.speech_secs.and_then(|s| s.parse().ok()),
                        wpm: p.wpm.and_then(|w| w.parse().ok()),
                    });
                }
            }
        }
    }
    stats
}

fn marker_has_real_prefix(prefix: &str) -> bool {
    let trimmed = prefix.trim();
    // Nothing but the trailing level word (with an optional ISO-8601-ish
    // timestamp before it) may precede the marker. Whitespace count varies
    // (tracing pads the level), so split on any run of it.
    if trimmed.is_empty() || trimmed == "INFO" {
        return true;
    }
    let mut words = trimmed.split_whitespace();
    let level = words.next_back();
    if level != Some("INFO") {
        return false;
    }
    words.all(|word| {
        word.chars()
            .all(|c| c.is_ascii_digit() || ".-T:+Z".contains(c))
    })
}

fn strip_ansi(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // Skip escape sequences up to the terminating letter.
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

pub(crate) async fn run_stats(
    config: &config::Config,
    lines: usize,
    format: &str,
) -> anyhow::Result<()> {
    let _ = config;
    let (messages, warning) = read_journal()?;
    let stats = collect_stats(&messages);
    if stats.is_empty() {
        println!("No dictation stats found in the journal.");
        println!();
        println!("Stats accumulate per dictation once the daemon logs them; make");
        println!("sure you're running a build that includes DictationStats logging.");
        println!("If stats disappear across reboots, enable persistent journald");
        println!("(Storage=persistent in journald.conf).");
        if let Some(warn) = warning {
            eprintln!("{}", warn);
        }
        return Ok(());
    }

    if format == "json" {
        let json = serde_json::to_string_pretty(&stats)?;
        println!("{}", json);
        return Ok(());
    }

    let spm: Vec<f32> = stats.iter().filter_map(|s| s.wpm).collect();
    // Only entries with real VAD speech count toward "total speech"; audio
    // fallback keeps the per-row WPM meaningful but isn't speech-on-mic.
    let total_speech: f32 = stats.iter().filter_map(|s| s.speech_secs).sum();
    let speech_count = stats.iter().filter(|s| s.speech_secs.is_some()).count();
    let avg_wpm = if spm.is_empty() {
        None
    } else {
        Some(spm.iter().sum::<f32>() / spm.len() as f32)
    };

    println!("Dictation stats ({} recorded{})", stats.len(), {
        if let Some(warning) = &warning {
            format!(", {}", warning.trim_end_matches('.'))
        } else {
            String::new()
        }
    });
    println!();
    if let Some(avg) = avg_wpm {
        println!("Average WPM: {:.0} across {} dictations", avg, spm.len());
    }
    if speech_count > 0 {
        println!(
            "Total speech: {:.1}s across {}/{} dictations (VAD)",
            total_speech,
            speech_count,
            stats.len()
        );
    } else {
        println!("Total speech: n/a (VAD did not run for any dictation)");
    }
    println!();
    println!("Recent dictations:");
    for stat in stats.iter().take(lines).rev() {
        let secs = stat.speech_secs.unwrap_or(stat.audio_secs);
        let wpm = stat
            .wpm
            .map(|w| format!("{:.0}", w))
            .unwrap_or_else(|| "-".to_string());
        println!(
            "{}  {:>4} wpm  {:>3} words  {:>5.1}s",
            format_ts(stat.ts_micros),
            wpm,
            stat.words,
            secs
        );
    }
    Ok(())
}

fn format_ts(micros: u64) -> String {
    let seconds = (micros / 1_000_000) as i64;
    let nanos = ((micros % 1_000_000) * 1000) as u32;
    chrono::DateTime::from_timestamp(seconds, nanos)
        .map(|dt| {
            dt.with_timezone(&chrono::Local)
                .format("%m-%d %H:%M:%S")
                .to_string()
        })
        .unwrap_or_else(|| format!("{}", micros))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_with_speech_and_wpm() {
        let p =
            parse_stats_message("INFO DictationStats: words=12 audio=4.50s speech=3.80s wpm=190")
                .unwrap();
        assert_eq!(p.words, 12);
        assert_eq!(p.audio_secs, 4.5);
        assert_eq!(p.speech_secs, Some("3.80"));
        assert_eq!(p.wpm, Some("190"));
    }

    #[test]
    fn parses_missing_vad_as_none() {
        let p = parse_stats_message("INFO DictationStats: words=3 audio=2.00s speech=none wpm=90")
            .unwrap();
        assert_eq!(p.speech_secs, None);
        assert_eq!(p.wpm, Some("90"));
    }

    #[test]
    fn parses_too_short_as_none_wpm() {
        let p =
            parse_stats_message("INFO DictationStats: words=5 audio=0.05s speech=none wpm=none")
                .unwrap();
        assert_eq!(p.wpm, None);
    }

    #[test]
    fn rejects_other_log_lines() {
        assert!(parse_stats_message("INFO Transcribed: \"hello\"").is_none());
        assert!(parse_stats_message("INFO DictationStats: garbage").is_none());
    }

    #[test]
    fn collects_across_framed_messages() {
        let messages = vec![
            (222u64, "INFO DictationStats: words=2 audio=3.00s speech=2.00s wpm=60".to_string()),
            (111u64, "INFO Transcribed: \"hi\"\nINFO DictationStats: words=1 audio=1.00s speech=none wpm=60".to_string()),
        ];
        let stats = collect_stats(&messages);
        assert_eq!(stats.len(), 2);
        // Journal scan is newest-first; stats keep that order.
        assert_eq!(stats[0].ts_micros, 222);
        assert_eq!(stats[1].ts_micros, 111);
    }

    #[test]
    fn dictated_fake_stats_are_not_collected() {
        let messages = vec![(
            111u64,
            "INFO Transcribed: \"DictationStats: words=99 audio=9.00s speech=none wpm=660\"\nDEBUG Post-processing input: \"INFO DictationStats: words=98 audio=9.00s speech=none wpm=653\"\nINFO DictationStats: words=12 audio=4.00s speech=3.00s wpm=180"
                .to_string(),
        )];
        let stats = collect_stats(&messages);
        assert_eq!(stats.len(), 1);
        assert_eq!(stats[0].words, 12);
    }

    #[test]
    fn message_byte_array_is_decoded() {
        let msg = serde_json::json!(
            "INFO DictationStats: words=2 audio=1.00s speech=none wpm=120"
                .bytes()
                .collect::<Vec<u8>>()
        );
        assert_eq!(
            decode_message(&msg).unwrap(),
            "INFO DictationStats: words=2 audio=1.00s speech=none wpm=120"
        );
    }
}
