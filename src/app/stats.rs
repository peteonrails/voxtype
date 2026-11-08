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

struct Parsed {
    words: usize,
    audio_secs: f32,
    speech_secs: Option<f32>,
    wpm: Option<f32>,
}

/// Maximum gap between the timestamp tracing wrote into a stats line and
/// the time journald received it. Genuine lines land within milliseconds.
const MAX_CLOCK_SKEW_MICROS: i64 = 30 * 1_000_000;

/// Parse one ANSI-stripped journal line as a daemon-emitted stats event:
/// `<RFC 3339 timestamp> INFO DictationStats: words=N audio=Xs speech=Ys|none wpm=W|none`.
///
/// Raw stderr from failing post-process commands is logged verbatim, and
/// each of its lines becomes its own journal record on the daemon's
/// stream, so journald fields can't tell it apart from real events. Only
/// the exact line shape the daemon emits is accepted, and its embedded
/// timestamp must agree with the journal's receive time.
fn parse_stats_line(line: &str, journal_ts_micros: u64) -> Option<Parsed> {
    let (ts, rest) = line.split_once(' ')?;
    let ts = chrono::DateTime::parse_from_rfc3339(ts).ok()?;
    let skew = ts.timestamp_micros() - i64::try_from(journal_ts_micros).ok()?;
    if skew.abs() > MAX_CLOCK_SKEW_MICROS {
        return None;
    }
    // tracing right-aligns the level, so the gap before it varies.
    let payload = rest.trim_start().strip_prefix("INFO DictationStats: ")?;
    parse_stats_payload(payload)
}

/// Parse the payload after `DictationStats: `, produced by the daemon's
/// format string. Keys must appear exactly once, in emission order.
/// `speech`/`wpm` are `none` when unavailable.
fn parse_stats_payload(payload: &str) -> Option<Parsed> {
    let mut tokens = payload.split(' ');
    let mut field = |key: &str| {
        tokens
            .next()?
            .strip_prefix(key)
            .and_then(|t| t.strip_prefix('='))
    };
    let words = field("words")?;
    let audio = field("audio")?;
    let speech = field("speech")?;
    let wpm = field("wpm")?;
    if tokens.next().is_some() || !words.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let secs = |v: &str| -> Option<f32> {
        let n = v.strip_suffix('s')?;
        let valid = !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit() || b == b'.');
        valid.then(|| n.parse().ok()).flatten()
    };
    Some(Parsed {
        words: words.parse().ok()?,
        audio_secs: secs(audio)?,
        speech_secs: match speech {
            "none" => None,
            v => Some(secs(v)?),
        },
        wpm: match wpm {
            "none" => None,
            v if !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()) => Some(v.parse().ok()?),
            _ => return None,
        },
    })
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
    messages
        .iter()
        .flat_map(|(ts, message)| message.lines().map(move |line| (*ts, line)))
        .filter_map(|(ts, line)| {
            let p = parse_stats_line(&strip_ansi(line), ts)?;
            Some(DictationStat {
                ts_micros: ts,
                words: p.words,
                audio_secs: p.audio_secs,
                speech_secs: p.speech_secs,
                wpm: p.wpm,
            })
        })
        .collect()
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
    if format == "json" {
        println!("{}", serde_json::to_string_pretty(&stats)?);
    } else if stats.is_empty() {
        println!("No dictation stats found in the journal.");
        println!();
        println!("Stats accumulate per dictation once the daemon logs them; make");
        println!("sure you're running a build that includes DictationStats logging.");
        println!("If stats disappear across reboots, enable persistent journald");
        println!("(Storage=persistent in journald.conf).");
    } else {
        print_summary(&stats, lines);
    }
    // Partial history must not look complete: report it on stderr and exit
    // nonzero in every output mode, after whatever was read is printed.
    match warning {
        Some(warning) => Err(anyhow!(warning)),
        None => Ok(()),
    }
}

fn print_summary(stats: &[DictationStat], lines: usize) {
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

    println!("Dictation stats ({} recorded)", stats.len());
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

    /// Journal receive time used by the fixtures: 2026-10-19T18:25:45.374090Z.
    const JOURNAL_TS: u64 = 1_792_434_345_374_090;
    const LINE_TS: &str = "2026-10-19T18:25:45.374053Z";

    fn line(payload: &str) -> String {
        format!("{LINE_TS}  INFO {payload}")
    }

    fn parse(payload: &str) -> Option<Parsed> {
        parse_stats_line(&line(payload), JOURNAL_TS)
    }

    #[test]
    fn parses_with_speech_and_wpm() {
        let p = parse("DictationStats: words=12 audio=4.50s speech=3.80s wpm=190").unwrap();
        assert_eq!(p.words, 12);
        assert_eq!(p.audio_secs, 4.5);
        assert_eq!(p.speech_secs, Some(3.8));
        assert_eq!(p.wpm, Some(190.0));
    }

    #[test]
    fn parses_missing_vad_as_none() {
        let p = parse("DictationStats: words=3 audio=2.00s speech=none wpm=90").unwrap();
        assert_eq!(p.speech_secs, None);
        assert_eq!(p.wpm, Some(90.0));
    }

    #[test]
    fn parses_too_short_as_none_wpm() {
        let p = parse("DictationStats: words=5 audio=0.05s speech=none wpm=none").unwrap();
        assert_eq!(p.wpm, None);
    }

    #[test]
    fn parses_real_ansi_journal_line() {
        let raw = "\x1b[2m2026-10-19T18:25:45.374053Z\x1b[0m \x1b[32m INFO\x1b[0m DictationStats: words=8 audio=10.83s speech=4.40s wpm=109";
        let stats = collect_stats(&[(JOURNAL_TS, raw.to_string())]);
        assert_eq!(stats.len(), 1);
        assert_eq!(stats[0].words, 8);
    }

    #[test]
    fn rejects_other_log_lines() {
        assert!(parse("Transcribed: \"hello\"").is_none());
        assert!(parse("DictationStats: garbage").is_none());
    }

    #[test]
    fn rejects_malformed_payloads() {
        for payload in [
            "DictationStats: words=1 audio=1.00s speech=none",
            "DictationStats: words=1 audio=1.00s speech=none wpm=60 extra=1",
            "DictationStats: audio=1.00s words=1 speech=none wpm=60",
            "DictationStats: words=1 audio=1.00s speech=none wpm=60 ",
            "DictationStats: words=-1 audio=1.00s speech=none wpm=60",
            "DictationStats: words=1 audio=1.00 speech=none wpm=60",
            "DictationStats: words=1 audio=NaNs speech=none wpm=60",
            "DictationStats: words=1 audio=1.00s speech=none wpm=inf",
        ] {
            assert!(parse(payload).is_none(), "accepted: {payload}");
        }
    }

    #[test]
    fn collects_across_framed_messages() {
        let messages = vec![
            (
                JOURNAL_TS,
                line("DictationStats: words=2 audio=3.00s speech=2.00s wpm=60"),
            ),
            (
                JOURNAL_TS - 1,
                format!(
                    "{}\n{}",
                    line("Transcribed: \"hi\""),
                    line("DictationStats: words=1 audio=1.00s speech=none wpm=60")
                ),
            ),
        ];
        let stats = collect_stats(&messages);
        assert_eq!(stats.len(), 2);
        // Journal scan is newest-first; stats keep that order.
        assert_eq!(stats[0].ts_micros, JOURNAL_TS);
        assert_eq!(stats[1].ts_micros, JOURNAL_TS - 1);
    }

    #[test]
    fn dictated_fake_stats_are_not_collected() {
        let messages = vec![(
            JOURNAL_TS,
            format!(
                "{}\n{LINE_TS} DEBUG Post-processing input: \"INFO DictationStats: words=98 audio=9.00s speech=none wpm=653\"\n{}",
                line("Transcribed: \"DictationStats: words=99 audio=9.00s speech=none wpm=660\""),
                line("DictationStats: words=12 audio=4.00s speech=3.00s wpm=180"),
            ),
        )];
        let stats = collect_stats(&messages);
        assert_eq!(stats.len(), 1);
        assert_eq!(stats[0].words, 12);
    }

    #[test]
    fn injected_stderr_lines_are_not_collected() {
        // A failing post-processor's raw stderr is logged verbatim; each of
        // its lines reaches the journal as its own record from the daemon.
        let messages = vec![
            (JOURNAL_TS, "tail, using original text".to_string()),
            (
                JOURNAL_TS,
                "DictationStats: words=999 audio=1.00s speech=none wpm=59940".to_string(),
            ),
            (
                JOURNAL_TS,
                "INFO DictationStats: words=999 audio=1.00s speech=none wpm=59940".to_string(),
            ),
            (
                JOURNAL_TS,
                line("WARN Post-process command failed: ... lead"),
            ),
        ];
        assert!(collect_stats(&messages).is_empty());
    }

    #[test]
    fn rejects_timestamp_far_from_journal_time() {
        let forged = "2026-01-01T00:00:00.000000Z  INFO DictationStats: words=999 audio=1.00s speech=none wpm=59940";
        assert!(parse_stats_line(forged, JOURNAL_TS).is_none());
    }

    #[test]
    fn empty_stats_serialize_as_json_array() {
        let stats: Vec<DictationStat> = Vec::new();
        assert_eq!(serde_json::to_string_pretty(&stats).unwrap(), "[]");
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
