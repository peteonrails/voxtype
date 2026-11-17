//! Import a folder written by Omarchy Meeting Recorder, so people who switch
//! keep their history.
//!
//! The format was read from Meeting Recorder's source (`src/meeting.rs`,
//! `src/export.rs`, `src/transcribe.rs`), not from folders it wrote:
//!
//! - `<title>.meeting-recorder`: JSON with `title`, `started_at` (Unix
//!   seconds), `duration_secs`, `format` (`mono`, `stereo` or `separate`),
//!   `speakers` (display names) and `labels` (the transcription's labels,
//!   `You`, `Remote 2`, `Speaker 1`, in the same order), `model` and
//!   `chapters` (`start_ms`, `title`)
//! - `transcript.md`: one `**[mm:ss] Name:** text` line per segment, with
//!   the display name (a rename rewrites the file)
//! - audio: `.tracks/mic.ogg` and `.tracks/computer.ogg` (mono, always both),
//!   else `mic.ogg` and `computer.ogg`, else `audio.ogg` (stereo with the
//!   mic on the left, or mono)
//! - `notes.md`, when there were notes
//!
//! The transcript is taken as it is, not transcribed again; `meeting
//! retranscribe` can do that afterwards with any voxtype engine. Chapters
//! become the meeting's summary, with an empty overview.

use crate::config::Config;
use crate::meeting::data::{Chapter, MeetingSummary};
use crate::meeting::import::{
    keep_audio, meeting_config_for, Channels, Ffmpeg, ImportError, ImportOutcome, ImportRequest,
    Progress, Side, Source,
};
use crate::meeting::manifest::MeetingContext;
use crate::meeting::{
    hook, AudioSource, MeetingMetadata, MeetingStatus, MeetingStorage, Transcript,
    TranscriptSegment,
};
use chrono::{DateTime, Utc};
use std::path::{Path, PathBuf};

/// Meeting Recorder's manifest extension.
pub const EXTENSION: &str = "meeting-recorder";

/// The `.meeting-recorder` file in `dir`, if it is a Meeting Recorder folder.
pub fn manifest_in(dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .find(|p| p.extension().is_some_and(|e| e == EXTENSION) && p.is_file())
}

/// One `**[mm:ss] Name:** text` line: start, name, text.
fn parse_line(line: &str) -> Option<(u64, String, String)> {
    let (time, rest) = line.strip_prefix("**[")?.split_once("] ")?;
    let (name, text) = rest.split_once(":** ")?;
    let start = crate::meeting::playback::parse_time(time).ok()?;
    Some((start, name.trim().to_string(), text.trim().to_string()))
}

/// The side a Meeting Recorder label belongs to: `You` and `You 2` are the
/// microphone, `Remote` and `Remote 1` the computer audio, anything else
/// (an import's `Speaker 1`) neither.
fn side_of(label: &str) -> AudioSource {
    let on = |side: &str| {
        label == side
            || label
                .strip_prefix(side)
                .and_then(|r| r.strip_prefix(' '))
                .is_some_and(|n| n.parse::<u32>().is_ok())
    };
    if on("You") {
        AudioSource::Microphone
    } else if on("Remote") {
        AudioSource::Loopback
    } else {
        AudioSource::Unknown
    }
}

/// The audio to keep, best first: the hidden per-side tracks, the separate
/// files, then `audio.ogg` (stereo is mic left, computer right).
fn audio_source(dir: &Path, format: &str, duration_ms: u64) -> Option<Source> {
    let pair = |a: PathBuf, b: PathBuf| {
        (a.is_file() && b.is_file()).then(|| Source {
            inputs: vec![a, b],
            channels: Channels::Mapped(vec![Side::Mic, Side::Loopback]),
            duration_ms,
            start_ms: 0,
        })
    };
    pair(
        dir.join(".tracks/mic.ogg"),
        dir.join(".tracks/computer.ogg"),
    )
    .or_else(|| pair(dir.join("mic.ogg"), dir.join("computer.ogg")))
    .or_else(|| {
        let audio = dir.join("audio.ogg");
        audio.is_file().then(|| Source {
            inputs: vec![audio],
            channels: if format == "stereo" {
                Channels::Mapped(vec![Side::Mic, Side::Loopback])
            } else {
                Channels::Mono
            },
            duration_ms,
            start_ms: 0,
        })
    })
}

/// Import the Meeting Recorder folder `request.file`. Only `title` of the
/// request applies; the transcript is the one Meeting Recorder made.
pub async fn import(
    config: &Config,
    request: &ImportRequest,
    progress: &mut dyn FnMut(&Progress),
) -> Result<ImportOutcome, ImportError> {
    let dir = &request.file;
    let unreadable = |reason: String| ImportError::Unreadable {
        path: dir.display().to_string(),
        reason,
    };
    let manifest_path =
        manifest_in(dir).ok_or_else(|| unreadable("no .meeting-recorder file in it".into()))?;
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&manifest_path)?)
            .map_err(|e| unreadable(format!("{}: {e}", manifest_path.display())))?;
    let ff = Ffmpeg::detect()?;

    let title = request
        .title
        .clone()
        .or_else(|| manifest["title"].as_str().map(str::to_string))
        .unwrap_or_else(|| "Meeting Recorder meeting".to_string());
    let started_at = manifest["started_at"]
        .as_i64()
        .and_then(|secs| DateTime::<Utc>::from_timestamp(secs, 0))
        .ok_or_else(|| unreadable("its manifest has no started_at".into()))?;
    let duration_ms = manifest["duration_secs"].as_u64().unwrap_or(0) * 1000;
    let strings = |key: &str| -> Vec<String> {
        manifest[key]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    };
    // Display name to label, from the two parallel lists.
    let (names, labels) = (strings("speakers"), strings("labels"));
    let label_of = |name: &str| -> String {
        names
            .iter()
            .position(|n| n == name)
            .and_then(|i| labels.get(i).cloned())
            .unwrap_or_else(|| name.to_string())
    };

    let markdown = std::fs::read_to_string(dir.join("transcript.md"))
        .map_err(|e| unreadable(format!("transcript.md: {e}")))?;
    let lines: Vec<(u64, String, String)> = markdown.lines().filter_map(parse_line).collect();
    let mut transcript = Transcript::new();
    for (i, (start, name, text)) in lines.iter().enumerate() {
        let end = lines
            .get(i + 1)
            .map(|(next, _, _)| *next)
            .unwrap_or(duration_ms)
            .max(*start);
        let label = label_of(name);
        let mut segment = TranscriptSegment::new(i as u32, *start, end, text.clone(), 0);
        segment.source = side_of(&label);
        segment.speaker_id = Some(label);
        transcript.add_segment(segment);
    }
    transcript.total_chunks = 1;

    let storage = MeetingStorage::open(meeting_config_for(config, None).storage)?;
    let mut metadata = MeetingMetadata::new(Some(title.clone()));
    metadata.started_at = started_at;
    metadata.ended_at = Some(started_at + chrono::Duration::milliseconds(duration_ms as i64));
    metadata.duration_secs = Some(duration_ms / 1000);
    metadata.status = MeetingStatus::Completed;
    metadata.chunk_count = 1;
    metadata.model = Some("whisper".to_string());
    let id = metadata.id;
    let folder = storage.create_meeting(&metadata)?;
    metadata.storage_path = Some(folder.clone());

    let mut report = |state: &str| {
        progress(&Progress {
            meeting_id: id.to_string(),
            operation: "import".to_string(),
            state: state.to_string(),
            chunks_done: 1,
            chunks_total: 1,
            position_ms: duration_ms,
            duration_ms,
        })
    };
    let audio_files =
        match audio_source(dir, manifest["format"].as_str().unwrap_or(""), duration_ms) {
            Some(source) => keep_audio(&ff, &source, &folder).unwrap_or_else(|e| {
                tracing::warn!("Could not keep the Meeting Recorder audio: {}", e);
                Vec::new()
            }),
            None => Vec::new(),
        };

    storage.save_transcript(&id, &transcript)?;
    for (name, label) in names.iter().zip(&labels) {
        if name != label {
            // A label for a speaker with no lines is refused; nothing lost.
            let _ = storage.set_speaker_label(&id, label, name);
        }
    }
    if let Ok(notes) = std::fs::read_to_string(dir.join("notes.md")) {
        if !notes.trim().is_empty() {
            storage.set_notes(&id, &notes)?;
        }
    }
    let chapters: Vec<Chapter> = manifest["chapters"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| {
            Some(Chapter {
                start_ms: c["start_ms"].as_u64()?,
                title: c["title"].as_str()?.to_string(),
            })
        })
        .collect();
    if !chapters.is_empty() {
        storage.save_summary(
            &id,
            &MeetingSummary {
                summary: String::new(),
                key_points: Vec::new(),
                action_items: Vec::new(),
                decisions: Vec::new(),
                chapters,
                generated_at: Utc::now(),
                model: manifest["chapters_by"].as_str().map(str::to_string),
            },
        )?;
    }
    let model = manifest["model"].as_str().map(str::to_string);
    storage.set_context(
        &id,
        &MeetingContext {
            engine: Some("whisper".to_string()),
            model: model.clone(),
            diarization: None,
            voxtype_version: Some(env!("CARGO_PKG_VERSION").to_string()),
            // Meeting Recorder does not record its echo handling.
            echo_cancel: None,
        },
    )?;
    metadata.audio_retained = !audio_files.is_empty();
    storage.update_meeting(&metadata)?;
    report("done");

    hook::PostMeeting {
        storage: meeting_config_for(config, None).storage,
        meeting_id: id,
        settings: hook::PostMeetingSettings::from_config(config),
    }
    .run()
    .await;

    Ok(ImportOutcome {
        meeting_id: id.to_string(),
        title,
        folder,
        started_at,
        duration_ms,
        channels: "meeting-recorder".to_string(),
        segments: transcript.segments.len(),
        engine: "whisper".to_string(),
        model: model.unwrap_or_default(),
        audio_files,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meeting::import::tests::{config_in, ffmpeg_present, wav};
    use crate::meeting::MeetingId;

    #[test]
    fn transcript_lines_parse() {
        assert_eq!(
            parse_line("**[01:23] Dana:** We ship Friday."),
            Some((83_000, "Dana".to_string(), "We ship Friday.".to_string()))
        );
        assert_eq!(
            parse_line("**[1:02:03] Remote 2:** Yes: agreed."),
            Some((
                3_723_000,
                "Remote 2".to_string(),
                "Yes: agreed.".to_string()
            ))
        );
        assert_eq!(parse_line("- **Date:** today"), None);
        assert_eq!(parse_line("## Transcript"), None);
        assert_eq!(side_of("You"), AudioSource::Microphone);
        assert_eq!(side_of("You 2"), AudioSource::Microphone);
        assert_eq!(side_of("Remote 1"), AudioSource::Loopback);
        assert_eq!(side_of("Speaker 1"), AudioSource::Unknown);
        assert_eq!(side_of("Youssef"), AudioSource::Unknown);
    }

    /// A folder laid out as Meeting Recorder's source writes one.
    fn fixture(dir: &Path) {
        std::fs::create_dir_all(dir.join(".tracks")).unwrap();
        let ogg = |name: &str, spans: &[(f32, f32)]| {
            let w = dir.join(format!("{name}.wav"));
            wav(&w, 12.0, &[spans]);
            let out = std::process::Command::new("ffmpeg")
                .args(["-v", "error", "-y", "-i"])
                .arg(&w)
                .args(["-c:a", "libopus", "-b:a", "64k"])
                .arg(dir.join(".tracks").join(format!("{name}.ogg")))
                .status()
                .unwrap();
            assert!(out.success());
            std::fs::remove_file(w).unwrap();
        };
        ogg("mic", &[(1.0, 3.0)]);
        ogg("computer", &[(4.0, 6.0), (8.0, 10.0)]);
        std::fs::write(
            dir.join("Launch sync.meeting-recorder"),
            serde_json::json!({
                "app": "Omarchy Meeting Recorder", "version": 1,
                "title": "Launch sync", "started_at": 1_762_095_600i64, "duration_secs": 12,
                "format": "mono", "language": "en",
                "speakers": ["Pete", "Dana", "Remote 2"],
                "labels": ["You", "Remote 1", "Remote 2"],
                "imported": null, "speaker_count": null, "model": "large-v3-turbo",
                "chapters": [{"start_ms": 0, "title": "Intro"}, {"start_ms": 8000, "title": "Dates"}],
                "chapters_by": "claude"
            })
            .to_string(),
        )
        .unwrap();
        std::fs::write(
            dir.join("transcript.md"),
            "# Launch sync\n\n- **Date:** 2 November 2025\n- **Duration:** 00:12\n\n\
             ## Transcript\n\n\
             **[00:01] Pete:** Morning all.\n\n\
             **[00:04] Dana:** Morning. The build is green.\n\n\
             **[00:08] Remote 2:** We ship Friday.\n\n",
        )
        .unwrap();
        std::fs::write(dir.join("notes.md"), "Ask about Friday.\n").unwrap();
    }

    #[tokio::test]
    async fn a_meeting_recorder_folder_imports_with_names_audio_and_chapters() {
        if !ffmpeg_present() {
            return;
        }
        let tmp = tempfile::TempDir::new().unwrap();
        let mr = tmp.path().join("202511021500 Launch sync");
        fixture(&mr);
        assert!(manifest_in(&mr).is_some());
        let config = config_in(&tmp.path().join("meetings"));
        let request = ImportRequest {
            file: mr.clone(),
            ..Default::default()
        };
        let outcome = crate::meeting::import::import(&config, &request, None, &mut |_| {})
            .await
            .unwrap();
        assert_eq!(outcome.title, "Launch sync");
        assert_eq!(outcome.channels, "meeting-recorder");
        assert_eq!(outcome.started_at.timestamp(), 1_762_095_600);
        assert_eq!(outcome.model, "large-v3-turbo");
        assert_eq!(outcome.segments, 3);
        assert_eq!(
            outcome.audio_files,
            ["mic.opus", "loopback.opus", "mixed.opus"].map(|f| f.replace(
                ".opus",
                &format!(
                    ".{}",
                    crate::meeting::retained_audio::Encoder::detect()
                        .format()
                        .extension()
                )
            ))
        );

        let storage = MeetingStorage::open(meeting_config_for(&config, None).storage).unwrap();
        let id = MeetingId::parse(&outcome.meeting_id).unwrap();
        let data = storage.load_meeting_data(&id).unwrap();
        let lines: Vec<(String, String, AudioSource, u64, u64)> = data
            .transcript
            .segments
            .iter()
            .map(|s| {
                (
                    s.speaker_key(),
                    s.speaker_display(),
                    s.source,
                    s.start_ms,
                    s.end_ms,
                )
            })
            .collect();
        assert_eq!(
            lines,
            [
                (
                    "You".into(),
                    "Pete".into(),
                    AudioSource::Microphone,
                    1_000,
                    4_000
                ),
                (
                    "Remote 1".into(),
                    "Dana".into(),
                    AudioSource::Loopback,
                    4_000,
                    8_000
                ),
                (
                    "Remote 2".into(),
                    "Remote 2".into(),
                    AudioSource::Loopback,
                    8_000,
                    12_000
                ),
            ]
        );
        assert_eq!(data.notes.as_deref(), Some("Ask about Friday.\n"));
        let summary = storage.get_summary(&id).unwrap().unwrap();
        let chapters: Vec<(u64, &str)> = summary
            .chapters
            .iter()
            .map(|c| (c.start_ms, c.title.as_str()))
            .collect();
        assert_eq!(chapters, [(0, "Intro"), (8_000, "Dates")]);
        assert_eq!(data.metadata.duration_secs, Some(12));
        assert_eq!(data.metadata.status, MeetingStatus::Completed);
        let m = crate::meeting::manifest::Manifest::read(&outcome.folder).unwrap();
        assert_eq!(m.model.as_deref(), Some("large-v3-turbo"));
        assert!(m.audio_retained);
        // The source folder is left as it was.
        assert!(mr.join(".tracks/mic.ogg").is_file());
    }
}
