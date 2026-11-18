//! Dictated and transcribed text stays out of the log at the default level.
//!
//! The daemon logs at info, and its systemd service logs to the journal,
//! which keeps what it is given. A log line carrying the user's words would
//! retain every dictation there, whatever they chose for history, failed
//! recordings and undelivered text. So the words are logged at debug only,
//! and the default level reports lengths.
//!
//! The dictation path is covered where it runs, by tests that capture real
//! log output: `daemon::tests::dictated_text_stays_out_of_the_log_at_the_default_level`
//! (recognition, cleanup, post-processing, file output),
//! `transcribe::tests::a_finished_transcription_is_logged_without_its_text_at_info`
//! (the line every engine reports its result through), and their
//! neighbours in `output::fallback`, `output::post_process`,
//! `meeting::assistant::summarize` and `state`.
//!
//! Those tests cannot reach every log call: most engines need a model to
//! run, and the output drivers type into the desktop. The checks here are
//! the wider net, over the source of every info, warn and error call. They
//! go by the names this code base gives to text, so they catch the usual
//! way a leak comes back (a `{:?}` of the transcript, a preview of its first
//! characters), not every possible one.

use regex::Regex;
use std::path::{Path, PathBuf};

const SOURCE_ROOTS: &[&str] = &["src", "crates/gguf-worker/src"];

/// Names this code base gives to bindings and fields that hold what the
/// user said, at any stage between the engine and the output.
const TEXT_BINDINGS: &[&str] = &[
    "text",
    "final_text",
    "processed_text",
    "combined",
    "collapsed",
    "transcript",
    "partial",
    "final_tail",
    "preview",
    "recent_context",
    // Dictation history: an entry as heard, as typed and as corrected.
    "raw_text",
    "corrected_text",
    "best_text",
];

/// Calls that use one of those names for something that is not the user's
/// words: the file, a fragment of the call, and why it is fine.
const NOT_THE_USERS_WORDS: &[(&str, &str, &str)] = &[
    (
        "src/meeting/calendar/mod.rs",
        "Calendar link not fetched",
        "`text` is the fetch error, which names the calendar host",
    ),
    (
        "src/config/load.rs",
        "is not a language code and is ignored",
        "`config.text.language` is the [text] section's language key, as written in the config file",
    ),
];

struct LogCall {
    file: String,
    line: usize,
    source: String,
}

fn rust_files(dir: &Path, found: &mut Vec<PathBuf>) {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|entry| entry.unwrap().path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            rust_files(&path, found);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            found.push(path);
        }
    }
}

/// The length of the string or character literal that starts at `at`, or
/// zero when there is none (a lifetime's `'` is not a literal).
fn literal_len(bytes: &[u8], at: usize) -> usize {
    match bytes[at] {
        b'"' => {
            let mut end = at + 1;
            while bytes[end] != b'"' {
                end += if bytes[end] == b'\\' { 2 } else { 1 };
            }
            end + 1 - at
        }
        b'\'' => {
            let rest = &bytes[at + 1..];
            if rest.len() >= 3 && rest[0] == b'\\' && rest[2] == b'\'' {
                4
            } else if rest.len() >= 2 && rest[0] != b'\\' && rest[1] == b'\'' {
                3
            } else {
                0
            }
        }
        _ => 0,
    }
}

/// Every info, warn and error call outside the test modules, whole.
fn default_level_log_calls() -> Vec<LogCall> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let call = Regex::new(r"\b(?:tracing::)?(?:info|warn|error)!\(").unwrap();
    let tests = Regex::new(r"(?m)^[ \t]*#\[cfg\(test\)\][ \t]*\n[ \t]*(?:pub(?:\(crate\))? )?mod ")
        .unwrap();
    let mut files = Vec::new();
    for dir in SOURCE_ROOTS {
        rust_files(&root.join(dir), &mut files);
    }

    let mut calls = Vec::new();
    for path in files {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if matches!(name.as_str(), "tests.rs" | "testing.rs" | "test_support.rs") {
            continue;
        }
        let source = std::fs::read_to_string(&path).unwrap();
        let production = match tests.find(&source) {
            Some(found) => &source[..found.start()],
            None => &source,
        };
        let bytes = production.as_bytes();
        for found in call.find_iter(production) {
            let mut at = found.end();
            let mut depth = 1;
            while depth > 0 {
                let skip = literal_len(bytes, at);
                if skip > 0 {
                    at += skip;
                    continue;
                }
                match bytes[at] {
                    b'(' => depth += 1,
                    b')' => depth -= 1,
                    _ => {}
                }
                at += 1;
            }
            calls.push(LogCall {
                file: path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
                line: production[..found.start()].matches('\n').count() + 1,
                source: production[found.start()..at].to_string(),
            });
        }
    }
    calls
}

/// A call's code with the literals blanked out, and its string literals.
fn code_and_strings(call: &str) -> (String, Vec<String>) {
    let bytes = call.as_bytes();
    let mut code = String::new();
    let mut strings = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let skip = literal_len(bytes, at);
        if skip > 0 {
            if bytes[at] == b'"' {
                strings.push(call[at + 1..at + skip - 1].to_string());
            }
            code.push(' ');
            at += skip;
        } else {
            let ch = call[at..].chars().next().unwrap();
            code.push(ch);
            at += ch.len_utf8();
        }
    }
    (code, strings)
}

/// The text bindings a call logs the value of. Its length, whether it is
/// empty or set, and a comparison with another value say nothing of the
/// words, so those uses do not count.
fn text_logged_by(call: &str) -> Vec<&'static str> {
    let measured = Regex::new(
        r"\b[A-Za-z_]\w*(?:\.[A-Za-z_]\w*)*(?:\.(?:chars|split_whitespace|lines)\(\))?\.(?:len|count|is_empty|is_some|is_none)\(\)",
    )
    .unwrap();
    let compared = Regex::new(r"[\w.]+\s*(?:!=|==)\s*[\w.]+").unwrap();
    let captured = Regex::new(r"\{([A-Za-z_]\w*)\s*(?::[^{}]*)?\}").unwrap();
    let word = Regex::new(r"\b[A-Za-z_]\w*\b").unwrap();

    let (code, strings) = code_and_strings(call);
    let code = measured.replace_all(&code, " ");
    let code = compared.replace_all(&code, " ");
    let mut names: Vec<&str> = word.find_iter(&code).map(|m| m.as_str()).collect();
    for string in &strings {
        names.extend(
            captured
                .captures_iter(string)
                .map(|c| c.get(1).unwrap().as_str()),
        );
    }
    TEXT_BINDINGS
        .iter()
        .copied()
        .filter(|binding| names.contains(binding))
        .collect()
}

/// A preview of the first characters of something is how transcripts used
/// to be logged ("Transcription completed in 0.4s: \"the first fifty...\"").
fn previews_text(call: &str) -> bool {
    let (code, _) = code_and_strings(call);
    code.contains(".chars().take(") || code.contains(".chars().skip(")
}

fn allowed(call: &LogCall) -> bool {
    NOT_THE_USERS_WORDS
        .iter()
        .any(|(file, fragment, _)| call.file == *file && call.source.contains(fragment))
}

#[test]
fn no_log_call_at_the_default_level_carries_dictated_text() {
    let calls = default_level_log_calls();
    assert!(
        calls.len() > 400,
        "found only {} log calls: the scan is not reading the source",
        calls.len()
    );

    let mut leaks = Vec::new();
    for call in calls.iter().filter(|call| !allowed(call)) {
        let bindings = text_logged_by(&call.source);
        if !bindings.is_empty() || previews_text(&call.source) {
            leaks.push(format!(
                "{}:{} logs {}:\n    {}",
                call.file,
                call.line,
                if bindings.is_empty() {
                    "a preview of some text".to_string()
                } else {
                    bindings.join(", ")
                },
                call.source.split_whitespace().collect::<Vec<_>>().join(" ")
            ));
        }
    }
    assert!(
        leaks.is_empty(),
        "these calls put what the user said in the log at the default level, where the \
         journal keeps it. Log the length at info and the text at debug (see \
         transcribe::log_transcription), or, if the value is not the user's words, add the \
         call to NOT_THE_USERS_WORDS with the reason:\n{}",
        leaks.join("\n")
    );

    for (file, fragment, _) in NOT_THE_USERS_WORDS {
        assert!(
            calls
                .iter()
                .any(|call| call.file == *file && call.source.contains(fragment)),
            "NOT_THE_USERS_WORDS names a call that is gone: {file}: {fragment}"
        );
    }
}

/// The scan above is only worth something if it sees the lines that used to
/// leak. These are the calls this test was written against.
#[test]
fn the_scan_recognizes_the_ways_text_was_logged() {
    for leaking in [
        r#"tracing::info!("Transcribed: {:?}", text)"#,
        r#"tracing::info!("Combined eager transcription: {:?}", combined)"#,
        r#"info!("Transcribed: {text}")"#,
        r#"tracing::warn!(%text, "Soniox: unparseable message")"#,
        r#"tracing::info!("Collapsed a repeated phrase in a {} line: {:?}", source, collapsed)"#,
        r#"tracing::info!("Line: {}", segment.text)"#,
        r#"tracing::info!("Post-processing context: {:?}", recent_context)"#,
        r#"tracing::warn!("Could not index {}: {:?}", entry.id, entry.raw_text)"#,
        r#"tracing::info!("History: pasted {}", entry.best_text())"#,
        r#"tracing::info!("History: corrected to {corrected_text}")"#,
    ] {
        assert!(!text_logged_by(leaking).is_empty(), "{leaking}");
    }
    assert!(previews_text(
        r#"tracing::info!("Cohere transcription completed in {:.2}s: {:?}", secs, result.chars().take(80).collect::<String>())"#
    ));

    for fine in [
        r#"tracing::info!("Transcribed {} chars ({} engine)", text.chars().count(), engine)"#,
        r#"tracing::info!("Text pasted via clipboard + {} ({} chars)", keystroke, text.len())"#,
        r#"tracing::info!("Post-processed: changed: {}", result != processed_text)"#,
        r#"tracing::info!("Post-processing, has_context: {}", recent_context.is_some())"#,
        r#"tracing::info!("{} words down to {}", segment.text.split_whitespace().count(), collapsed.split_whitespace().count())"#,
        r#"tracing::warn!("Failed to append text: {}", stderr)"#,
        r#"tracing::info!("History: recorded {} ({} chars)", entry.id, entry.raw_text.chars().count())"#,
    ] {
        assert!(text_logged_by(fine).is_empty(), "{fine}");
        assert!(!previews_text(fine), "{fine}");
    }
}

/// The docs say what the log holds, so they change with it.
#[test]
fn the_docs_say_dictated_text_is_not_logged_by_default_and_how_to_see_it() {
    let root = env!("CARGO_MANIFEST_DIR");
    let read = |name: &str| std::fs::read_to_string(format!("{root}/docs/{name}")).unwrap();

    for (name, promise) in [
        ("CONFIGURATION.md", "voxtype does not log what you say"),
        ("USER_MANUAL.md", "voxtype does not log what you say"),
        (
            "TROUBLESHOOTING.md",
            "the log does not contain what you say",
        ),
    ] {
        let doc = read(name);
        assert!(
            doc.contains(promise),
            "{name} must say text is not logged by default"
        );
        assert!(
            doc.contains("Environment=RUST_LOG=voxtype=debug,warn") && doc.contains("voxtype -v"),
            "{name} must say how to turn on debug logging to see the text"
        );
        assert!(
            !doc.contains("[INFO] Transcribed: \""),
            "{name} shows the transcription in the log at info"
        );
    }

    // What the docs promise is what the daemon does.
    let daemon = std::fs::read_to_string(format!("{root}/src/daemon.rs")).unwrap();
    assert!(
        daemon.contains(r#"tracing::debug!("Transcribed: {:?}", text)"#)
            && daemon.contains(r#""Transcribed {} chars ({} engine)""#),
        "the daemon's transcription lines changed: bring the docs' examples of them \
         (\"Transcribed 42 chars (whisper engine)\", \"Transcribed: ...\" at debug) in line, \
         then this test"
    );
}
