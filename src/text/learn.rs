//! Phrase-level diff used by `voxtype learn`.
//!
//! Whitespace-split SequenceMatcher (Python `difflib` shape): only `replace`
//! opcodes become `(old phrase, new phrase)` pairs. Inserts and deletes are
//! ignored so a compositor selection that only adds or drops words does not
//! poison `[text.replacements]`.

use std::path::{Path, PathBuf};

/// Full-string similarity below this refuses to learn (random selection guard).
pub const MIN_SIMILARITY: f64 = 0.35;

/// Result of comparing a last transcript to corrected text.
#[derive(Debug, Clone, PartialEq)]
pub enum LearnDiff {
    /// Corrected text matches the transcript after whitespace trim.
    Identical,
    /// Character-level SequenceMatcher ratio is below [`MIN_SIMILARITY`].
    TooDifferent { ratio: f64 },
    /// Texts are similar but every change is an insert or delete.
    NoReplacements,
    /// One or more replace-opcode phrases to merge into config.
    Replacements(Vec<(String, String)>),
}

/// Compare `transcript` (last dictation) to `corrected` (edited selection).
pub fn diff_replacements(transcript: &str, corrected: &str) -> LearnDiff {
    let transcript = transcript.trim();
    let corrected = corrected.trim();

    if transcript == corrected {
        return LearnDiff::Identical;
    }

    let ratio = sequence_ratio(transcript.chars(), corrected.chars());
    if ratio < MIN_SIMILARITY {
        return LearnDiff::TooDifferent { ratio };
    }

    let old_tokens: Vec<&str> = transcript.split_whitespace().collect();
    let new_tokens: Vec<&str> = corrected.split_whitespace().collect();
    let pairs = replace_pairs(&old_tokens, &new_tokens);
    if pairs.is_empty() {
        LearnDiff::NoReplacements
    } else {
        LearnDiff::Replacements(pairs)
    }
}

/// Path of the last-transcript file the daemon writes after each dictation.
pub fn last_transcript_path() -> PathBuf {
    crate::config::Config::runtime_dir().join("last-transcript")
}

/// Persist the raw transcribed text so `voxtype learn` does not depend on journald.
pub fn record_last_transcript(text: &str) {
    if let Err(e) = write_last_transcript(&last_transcript_path(), text) {
        tracing::debug!("Failed to record last transcript: {}", e);
    }
}

fn write_last_transcript(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, text)
}

/// Last transcript: on-disk file first, then journald `Transcribed: "..."` lines.
pub fn load_last_transcript() -> Option<String> {
    load_last_transcript_from(&last_transcript_path()).or_else(load_last_transcript_from_journal)
}

fn load_last_transcript_from(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn load_last_transcript_from_journal() -> Option<String> {
    let output = std::process::Command::new("journalctl")
        .args([
            "--user",
            "-u",
            "voxtype",
            "--output=cat",
            "--no-pager",
            "-n",
            "500",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout.lines().rev().find_map(parse_transcribed_line)
}

/// Parse a tracing `Transcribed: {:?}` line into the original string.
pub fn parse_transcribed_line(line: &str) -> Option<String> {
    const MARKER: &str = "Transcribed: \"";
    let start = line.find(MARKER)? + MARKER.len();
    unescape_debug_str(&line[start..])
}

fn unescape_debug_str(s: &str) -> Option<String> {
    let mut out = String::new();
    let mut chars = s.chars().peekable();
    loop {
        match chars.next()? {
            '"' => return Some(out),
            '\\' => match chars.next()? {
                '"' => out.push('"'),
                '\\' => out.push('\\'),
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                '\'' => out.push('\''),
                '0' => out.push('\0'),
                'u' => {
                    if chars.next() != Some('{') {
                        return None;
                    }
                    let mut hex = String::new();
                    loop {
                        match chars.next()? {
                            '}' => break,
                            c => hex.push(c),
                        }
                    }
                    let cp = u32::from_str_radix(&hex, 16).ok()?;
                    out.push(char::from_u32(cp)?);
                }
                c => out.push(c),
            },
            c => out.push(c),
        }
    }
}

// ---------------------------------------------------------------------------
// SequenceMatcher (Ratcliff/Obershelp), matching Python difflib for opcodes
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tag {
    Equal,
    Replace,
    Insert,
    Delete,
}

struct Opcode {
    tag: Tag,
    a_start: usize,
    a_end: usize,
    b_start: usize,
    b_end: usize,
}

fn sequence_ratio<I, T>(a: I, b: I) -> f64
where
    I: IntoIterator<Item = T>,
    T: PartialEq,
{
    let a: Vec<T> = a.into_iter().collect();
    let b: Vec<T> = b.into_iter().collect();
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    let matches: usize = matching_blocks(&a, &b).into_iter().map(|m| m.2).sum();
    2.0 * matches as f64 / (a.len() + b.len()) as f64
}

fn replace_pairs(old_tokens: &[&str], new_tokens: &[&str]) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    for op in opcodes(old_tokens, new_tokens) {
        if op.tag != Tag::Replace {
            continue;
        }
        let a_len = op.a_end - op.a_start;
        let b_len = op.b_end - op.b_start;
        let n = a_len.min(b_len);
        if n == 0 {
            continue;
        }
        let old_phrase = old_tokens[op.a_start..op.a_start + n].join(" ");
        let new_phrase = new_tokens[op.b_start..op.b_start + n].join(" ");
        if old_phrase != new_phrase {
            pairs.push((old_phrase, new_phrase));
        }
    }
    pairs
}

fn opcodes<T: PartialEq>(a: &[T], b: &[T]) -> Vec<Opcode> {
    let mut i = 0;
    let mut j = 0;
    let mut answer = Vec::new();
    for (ai, bj, size) in matching_blocks(a, b) {
        if i < ai && j < bj {
            answer.push(Opcode {
                tag: Tag::Replace,
                a_start: i,
                a_end: ai,
                b_start: j,
                b_end: bj,
            });
        } else if i < ai {
            answer.push(Opcode {
                tag: Tag::Delete,
                a_start: i,
                a_end: ai,
                b_start: j,
                b_end: bj,
            });
        } else if j < bj {
            answer.push(Opcode {
                tag: Tag::Insert,
                a_start: i,
                a_end: ai,
                b_start: j,
                b_end: bj,
            });
        }
        i = ai + size;
        j = bj + size;
        if size > 0 {
            answer.push(Opcode {
                tag: Tag::Equal,
                a_start: ai,
                a_end: i,
                b_start: bj,
                b_end: j,
            });
        }
    }
    answer
}

fn matching_blocks<T: PartialEq>(a: &[T], b: &[T]) -> Vec<(usize, usize, usize)> {
    let mut acc = Vec::new();
    collect_matching_blocks(a, 0, a.len(), b, 0, b.len(), &mut acc);
    acc.push((a.len(), b.len(), 0));
    acc
}

fn collect_matching_blocks<T: PartialEq>(
    a: &[T],
    a0: usize,
    a1: usize,
    b: &[T],
    b0: usize,
    b1: usize,
    acc: &mut Vec<(usize, usize, usize)>,
) {
    let (i, j, size) = longest_match(a, a0, a1, b, b0, b1);
    if size == 0 {
        return;
    }
    if i > a0 && j > b0 {
        collect_matching_blocks(a, a0, i, b, b0, j, acc);
    }
    acc.push((i, j, size));
    if i + size < a1 && j + size < b1 {
        collect_matching_blocks(a, i + size, a1, b, j + size, b1, acc);
    }
}

fn longest_match<T: PartialEq>(
    a: &[T],
    a0: usize,
    a1: usize,
    b: &[T],
    b0: usize,
    b1: usize,
) -> (usize, usize, usize) {
    let mut best_i = a0;
    let mut best_j = b0;
    let mut best_size = 0usize;
    let width = b1 - b0;
    if width == 0 || a1 == a0 {
        return (best_i, best_j, best_size);
    }
    let mut prev = vec![0usize; width];
    let mut curr = vec![0usize; width];
    for (i, a_item) in a.iter().enumerate().take(a1).skip(a0) {
        for (bj, b_item) in b[b0..b1].iter().enumerate() {
            if a_item == b_item {
                let size = if bj == 0 { 1 } else { prev[bj - 1] + 1 };
                curr[bj] = size;
                if size > best_size {
                    best_i = i + 1 - size;
                    best_j = b0 + bj + 1 - size;
                    best_size = size;
                }
            } else {
                curr[bj] = 0;
            }
        }
        std::mem::swap(&mut prev, &mut curr);
        curr.fill(0);
    }
    (best_i, best_j, best_size)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn example_omaar_gielen() {
        let transcript = "Het omaar gielen nu, doet het niet.";
        let corrected = "Het Omarchy menu doet het niet.";
        match diff_replacements(transcript, corrected) {
            LearnDiff::Replacements(pairs) => {
                assert_eq!(
                    pairs,
                    vec![("omaar gielen".to_string(), "Omarchy menu".to_string())]
                );
            }
            other => panic!("expected replacements, got {other:?}"),
        }
    }

    #[test]
    fn identical_is_noop() {
        let text = "Het Omarchy menu doet het niet.";
        assert_eq!(diff_replacements(text, text), LearnDiff::Identical);
        assert_eq!(
            diff_replacements("  hello world  ", "hello world"),
            LearnDiff::Identical
        );
    }

    #[test]
    fn low_similarity_is_refused() {
        match diff_replacements(
            "Het omaar gielen nu, doet het niet.",
            "completely unrelated text about cooking pasta tonight",
        ) {
            LearnDiff::TooDifferent { ratio } => {
                assert!(
                    ratio < MIN_SIMILARITY,
                    "ratio {ratio} should be below {MIN_SIMILARITY}"
                );
            }
            other => panic!("expected TooDifferent, got {other:?}"),
        }
    }

    #[test]
    fn insert_only_yields_no_replacements() {
        assert_eq!(
            diff_replacements("hello world", "hello there world"),
            LearnDiff::NoReplacements
        );
    }

    #[test]
    fn delete_only_yields_no_replacements() {
        assert_eq!(
            diff_replacements("hello there world", "hello world"),
            LearnDiff::NoReplacements
        );
    }

    #[test]
    fn example_similarity_is_high_enough() {
        let ratio = sequence_ratio(
            "Het omaar gielen nu, doet het niet.".chars(),
            "Het Omarchy menu doet het niet.".chars(),
        );
        assert!((ratio - 0.7878).abs() < 0.01, "unexpected ratio {ratio}");
        assert!(ratio > MIN_SIMILARITY);
    }

    #[test]
    fn parse_transcribed_debug_line() {
        assert_eq!(
            parse_transcribed_line(r#"2026-08-30T18:40:21Z  INFO Transcribed: "hello world""#),
            Some("hello world".to_string())
        );
        assert_eq!(
            parse_transcribed_line(r#"Transcribed: "say \"hi\"""#),
            Some(r#"say "hi""#.to_string())
        );
        assert_eq!(parse_transcribed_line("no marker here"), None);
    }

    #[test]
    fn last_transcript_file_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("last-transcript");
        write_last_transcript(&path, "Het omaar gielen nu, doet het niet.").unwrap();
        assert_eq!(
            load_last_transcript_from(&path).as_deref(),
            Some("Het omaar gielen nu, doet het niet.")
        );
        write_last_transcript(&path, "   \n").unwrap();
        assert!(load_last_transcript_from(&path).is_none());
    }
}
