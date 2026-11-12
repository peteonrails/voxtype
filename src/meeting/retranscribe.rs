//! `voxtype meeting retranscribe`: transcribe a stored meeting again from
//! its retained audio, with another engine, model or language.
//!
//! The meeting keeps its ID, folder, notes, speaker names and summary. The
//! transcript it had is kept as `transcript.vN.json` (`v1` the first time),
//! and the new one takes its place.
//!
//! Per-line edits are keyed by segment ID, and a new transcription numbers
//! its segments afresh, so edits cannot simply be kept. Each old line is
//! matched to the new line it overlaps most in time, on the same side; an
//! annotation follows its line that way always, and speaker moves, text
//! corrections and deleted lines follow with `--keep-edits` (the default)
//! or are dropped with `--discard-edits`. An edit whose line has no
//! match is dropped either way, and the outcome says how many.

use crate::config::{Config, MeetingStartOptions};
use crate::meeting::import::{
    self, daemon_for, meeting_config_for, mono_diarization, Channels, Ffmpeg, ImportError,
    Progress, Side, Source,
};
use crate::meeting::manifest::{self, MeetingContext, TRANSCRIPT_JSON_FILE};
use crate::meeting::storage::SegmentEditCarry;
use crate::meeting::{
    retained_audio, AudioSource, MeetingStorage, StorageConfig, TranscriptSegment,
};
use crate::transcribe::Transcriber;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;

/// What `meeting retranscribe` asks for.
#[derive(Debug, Clone, Default)]
pub struct RetranscribeRequest {
    /// Meeting ID, a unique start of one, or `latest`
    pub meeting: String,
    pub options: MeetingStartOptions,
    pub diarization: Option<String>,
    /// Carry speaker moves, text edits and deleted lines over by time
    pub keep_edits: bool,
}

/// What `meeting retranscribe --json` prints at the end.
#[derive(Debug, Clone, Serialize)]
pub struct RetranscribeOutcome {
    pub meeting_id: String,
    /// The file the previous transcript was kept as, if there was one
    pub previous_transcript: Option<String>,
    pub segments: usize,
    pub engine: String,
    pub model: String,
    /// `mic,loopback`, `mic` or `mono`: how the retained audio was read
    pub channels: String,
    pub edits: SegmentEditCarry,
}

/// The new line each old line overlaps most, keyed by old segment ID.
///
/// A match needs the two lines on the same side (either may be unsided)
/// and an overlap of at least half the shorter line, so a correction is
/// not pasted onto a line that merely touches it.
pub fn map_segments(old: &[TranscriptSegment], new: &[TranscriptSegment]) -> HashMap<u32, u32> {
    let span = |s: &TranscriptSegment| (s.start_ms, s.end_ms.max(s.start_ms + 1));
    old.iter()
        .filter_map(|o| {
            let (os, oe) = span(o);
            new.iter()
                .filter(|n| {
                    o.source == n.source
                        || o.source == AudioSource::Unknown
                        || n.source == AudioSource::Unknown
                })
                .filter_map(|n| {
                    let (ns, ne) = span(n);
                    let overlap = oe.min(ne).saturating_sub(os.max(ns));
                    let shorter = (oe - os).min(ne - ns);
                    (overlap > 0 && overlap * 2 >= shorter).then_some((overlap, n.id))
                })
                .max_by_key(|&(overlap, id)| (overlap, std::cmp::Reverse(id)))
                .map(|(_, id)| (o.id, id))
        })
        .collect()
}

/// The retained audio to transcribe, as one source: the mic and loopback
/// tracks side by side when both were kept, the mic alone for a mic-only
/// meeting, otherwise the mixed track with no sides.
fn retained_source(ff: &Ffmpeg, dir: &std::path::Path) -> Result<Source, ImportError> {
    let files: HashMap<&str, String> = retained_audio::retained_files(dir).into_iter().collect();
    let (inputs, channels) = match (
        files.get(retained_audio::MIC),
        files.get(retained_audio::LOOPBACK),
        files.get(retained_audio::MIXED),
    ) {
        (Some(mic), Some(loopback), _) => (
            vec![dir.join(mic), dir.join(loopback)],
            Channels::Mapped(vec![Side::Mic, Side::Loopback]),
        ),
        (Some(mic), None, _) => (vec![dir.join(mic)], Channels::Mapped(vec![Side::Mic])),
        (None, _, Some(mixed)) => (vec![dir.join(mixed)], Channels::Mono),
        _ => return Err(ImportError::NoAudio),
    };
    let duration_ms = import::probe(ff, &inputs[0])?.duration_ms;
    Ok(Source {
        inputs,
        channels,
        duration_ms,
    })
}

/// Transcribe a stored meeting again. `resident` is a transcriber to use
/// instead of loading one (tests pass a fake). `progress` is called after
/// every chunk and once at the end.
pub async fn retranscribe(
    config: &Config,
    request: &RetranscribeRequest,
    resident: Option<Arc<dyn Transcriber>>,
    progress: &mut dyn FnMut(&Progress),
) -> Result<RetranscribeOutcome, ImportError> {
    let ff = Ffmpeg::detect()?;
    let meeting_config = meeting_config_for(config, request.diarization.as_deref());
    let storage = MeetingStorage::open(meeting_config.storage.clone())?;
    let id = storage.resolve_meeting_id(&request.meeting)?;
    let metadata = storage
        .get_meeting(&id)?
        .ok_or_else(|| crate::meeting::StorageError::NotFound(id.to_string()))?;
    let dir = storage.get_meeting_path(&id)?;
    let source = retained_source(&ff, &dir)?;
    let effective = crate::meeting::start::meeting_transcription_config(config, &request.options)
        .map_err(ImportError::Config)?;

    // Transcribe into a scratch store, so the meeting itself only changes
    // once the new transcript is complete.
    let scratch = tempfile::TempDir::new()?;
    let mut scratch_config = meeting_config.clone();
    scratch_config.storage = StorageConfig {
        storage_path: scratch.path().to_path_buf(),
        ..scratch_config.storage
    };
    if source.channels == Channels::Mono {
        mono_diarization(&mut scratch_config);
    }
    let diarization = scratch_config
        .diarization
        .as_ref()
        .map(|d| d.backend.clone());
    let chunk_secs = scratch_config.chunk_duration_secs;
    let vad_threshold = scratch_config.vad_threshold;
    let scratch_storage = MeetingStorage::open(scratch_config.storage.clone())?;
    let mut daemon = daemon_for(scratch_config, &effective, resident)?;
    let scratch_id = daemon
        .start_at(metadata.title.clone(), None, Some(metadata.started_at))
        .await?;

    let total = source
        .duration_ms
        .div_ceil(chunk_secs.max(1) as u64 * 1000)
        .max(1) as u32;
    let report = |state: &str, done: u32, position_ms: u64| Progress {
        meeting_id: id.to_string(),
        operation: "retranscribe".to_string(),
        state: state.to_string(),
        chunks_done: done,
        chunks_total: total.max(done),
        position_ms,
        duration_ms: source.duration_ms,
    };
    progress(&report("transcribing", 0, 0));
    let result = import::transcribe_source(
        &mut daemon,
        &ff,
        &source,
        chunk_secs,
        vad_threshold,
        &mut |done, position| progress(&report("transcribing", done, position)),
    )
    .await;
    let stopped = daemon.stop().await;
    if let Err(e) = result {
        progress(&report("failed", 0, 0));
        return Err(e);
    }
    stopped?;
    let new = scratch_storage.load_transcript(&scratch_id)?;

    // Keep the old transcript, tombstones and all, as the next version.
    let previous_transcript = if dir.join(TRANSCRIPT_JSON_FILE).is_file() {
        let n = manifest::transcript_versions(&dir)
            .last()
            .map_or(1, |(n, _)| n + 1);
        let name = format!("transcript.v{n}.json");
        std::fs::copy(dir.join(TRANSCRIPT_JSON_FILE), dir.join(&name))?;
        Some(name)
    } else {
        None
    };
    let old = match storage.load_transcript_all(&id) {
        Ok(t) => t.segments,
        Err(crate::meeting::StorageError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
            Vec::new()
        }
        Err(e) => return Err(e.into()),
    };
    let mapping = map_segments(&old, &new.segments);
    let edits = storage.remap_segment_edits(&id, &mapping, request.keep_edits)?;
    storage.save_transcript(&id, &new)?;
    // The scratch meeting named its speakers in the scratch store; the new
    // speaker IDs need the names here too.
    if config.meeting.auto_label_from_title {
        if let Some(me) = crate::meeting::title_labels::Me::resolve(None) {
            crate::meeting::title_labels::label_stored(&storage, &id, &me)?;
        }
    }

    let engine = format!("{:?}", effective.engine).to_lowercase();
    let model = effective.model_name().to_string();
    let mut context = storage.get_context(&id)?;
    context = MeetingContext {
        engine: Some(engine.clone()),
        model: Some(model.clone()),
        diarization,
        ..context
    };
    storage.set_context(&id, &context)?;
    let mut metadata = metadata;
    metadata.model = Some(engine.clone());
    metadata.chunk_count = new.total_chunks;
    storage.update_meeting(&metadata)?;

    let mut last = report("done", 0, source.duration_ms);
    last.chunks_done = last.chunks_total;
    progress(&last);
    Ok(RetranscribeOutcome {
        meeting_id: id.to_string(),
        previous_transcript,
        segments: storage.load_transcript(&id)?.segments.len(),
        engine,
        model,
        channels: source.channels.describe(),
        edits,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meeting::import::tests::{config_in, counting, ffmpeg_present, wav};
    use crate::meeting::import::{import, ImportRequest};
    use crate::meeting::MeetingId;

    fn seg(id: u32, start: u64, end: u64, source: AudioSource) -> TranscriptSegment {
        let mut s = TranscriptSegment::new(id, start, end, format!("s{id}"), 0);
        s.source = source;
        s
    }

    #[test]
    fn lines_map_to_the_new_line_they_overlap_most() {
        use AudioSource::{Loopback as L, Microphone as M};
        let old = [
            seg(0, 1_000, 3_000, M),
            seg(1, 5_000, 7_000, L),
            seg(2, 8_000, 8_400, M), // overlaps less than half of the shorter
            seg(3, 20_000, 21_000, M), // nothing new there
        ];
        let new = [
            seg(10, 900, 2_900, M),
            seg(11, 4_000, 6_000, L),
            seg(12, 5_500, 7_500, L),
            seg(13, 8_300, 9_000, M),
            seg(14, 20_000, 21_000, L), // the other side
        ];
        let map = map_segments(&old, &new);
        assert_eq!(map.get(&0), Some(&10));
        assert_eq!(map.get(&1), Some(&12), "1.5 s overlap beats 1 s");
        assert_eq!(map.get(&2), None);
        assert_eq!(map.get(&3), None);

        // An unsided (imported mono) transcript matches either side.
        let mono = [seg(20, 1_000, 3_000, AudioSource::Unknown)];
        assert_eq!(map_segments(&old, &mono).get(&0), Some(&20));
    }

    /// Import a stereo call, edit it, and transcribe it again.
    #[tokio::test]
    async fn retranscribing_versions_the_transcript_and_carries_edits() {
        if !ffmpeg_present() {
            return;
        }
        let dir = tempfile::TempDir::new().unwrap();
        let file = dir.path().join("call.wav");
        wav(&file, 12.0, &[&[(1.0, 3.0), (8.0, 9.0)], &[(5.0, 7.0)]]);
        let config = config_in(&dir.path().join("meetings"));
        let imported = import(
            &config,
            &ImportRequest {
                file,
                channels: Some("mic,loopback".to_string()),
                ..Default::default()
            },
            counting(),
            &mut |_| {},
        )
        .await
        .unwrap();
        let storage =
            MeetingStorage::open(meeting_config_for(&config, None).storage.clone()).unwrap();
        let id = MeetingId::parse(&imported.meeting_id).unwrap();
        let before = storage.load_transcript(&id).unwrap().segments;
        assert_eq!(before.len(), 3);
        let (you1, remote, you2) = (before[0].id, before[1].id, before[2].id);

        storage.set_speaker_label(&id, "Remote", "Dana").unwrap();
        storage.set_notes(&id, "Keep me.\n").unwrap();
        storage
            .annotate_segment(&id, remote, "Decision", true, true)
            .unwrap();
        storage.reassign_segment(&id, you2, "Dana", true).unwrap();
        storage
            .edit_segment_text(&id, you1, "Hello there", true)
            .unwrap();
        let old_json = std::fs::read_to_string(imported.folder.join("transcript.json")).unwrap();

        let mut seen = Vec::new();
        let request = RetranscribeRequest {
            meeting: "latest".to_string(),
            keep_edits: true,
            ..Default::default()
        };
        let outcome = retranscribe(&config, &request, counting(), &mut |p| seen.push(p.clone()))
            .await
            .unwrap();
        assert_eq!(outcome.meeting_id, imported.meeting_id);
        assert_eq!(
            outcome.previous_transcript.as_deref(),
            Some("transcript.v1.json")
        );
        assert_eq!(outcome.channels, "mic,loopback");
        assert_eq!(outcome.segments, 3);
        assert_eq!(
            outcome.edits,
            SegmentEditCarry {
                edits_carried: 2,
                annotations_carried: 1,
                ..Default::default()
            }
        );
        assert_eq!(
            std::fs::read_to_string(imported.folder.join("transcript.v1.json")).unwrap(),
            old_json
        );
        assert_eq!(seen.last().unwrap().operation, "retranscribe");
        assert_eq!(seen.last().unwrap().state, "done");

        // Names, notes, the annotation, the move and the correction carried.
        let data = storage.load_meeting_data(&id).unwrap();
        let lines: Vec<(String, String)> = data
            .transcript
            .segments
            .iter()
            .map(|s| (s.speaker_display(), s.text.clone()))
            .collect();
        assert_eq!(lines[0], ("You".to_string(), "Hello there".to_string()));
        assert_eq!(lines[1].0, "Dana");
        assert_eq!(lines[2].0, "Dana", "the moved line");
        assert_eq!(
            data.transcript.segments[1]
                .annotation
                .as_ref()
                .unwrap()
                .text,
            "Decision"
        );
        assert_eq!(data.notes.as_deref(), Some("Keep me.\n"));
        let m = crate::meeting::manifest::Manifest::read(&imported.folder).unwrap();
        assert!(m
            .files
            .iter()
            .any(|f| f.name == "transcript.v1.json" && f.kind == "transcript_previous"));

        // Again, discarding edits: the next version, moves and corrections
        // gone, the annotation still there.
        let request = RetranscribeRequest {
            meeting: imported.meeting_id.clone(),
            keep_edits: false,
            ..Default::default()
        };
        let outcome = retranscribe(&config, &request, counting(), &mut |_| {})
            .await
            .unwrap();
        assert_eq!(
            outcome.previous_transcript.as_deref(),
            Some("transcript.v2.json")
        );
        assert_eq!(outcome.edits.edits_discarded, 2);
        assert_eq!(outcome.edits.annotations_carried, 1);
        let data = storage.load_meeting_data(&id).unwrap();
        let speakers: Vec<String> = data
            .transcript
            .segments
            .iter()
            .map(|s| s.speaker_display())
            .collect();
        assert_eq!(speakers, ["You", "Dana", "You"]);
        assert_ne!(data.transcript.segments[0].text, "Hello there");
    }

    /// Retranscribing names the new speakers from the title, and keeps a
    /// name given by hand.
    #[tokio::test]
    async fn retranscribing_names_speakers_from_the_title() {
        if !ffmpeg_present() {
            return;
        }
        crate::meeting::title_labels::set_account_name_for_test(Some("Pete Jackson"));
        let dir = tempfile::TempDir::new().unwrap();
        let file = dir.path().join("call.wav");
        wav(&file, 12.0, &[&[(1.0, 3.0)], &[(5.0, 7.0)]]);
        let mut config = config_in(&dir.path().join("meetings"));
        // Imported before the setting was on, then retitled.
        config.meeting.auto_label_from_title = false;
        let imported = import(
            &config,
            &ImportRequest {
                file,
                title: Some("1:1 Pete/Chris".to_string()),
                channels: Some("mic,loopback".to_string()),
                ..Default::default()
            },
            counting(),
            &mut |_| {},
        )
        .await
        .unwrap();
        let storage =
            MeetingStorage::open(meeting_config_for(&config, None).storage.clone()).unwrap();
        let id = MeetingId::parse(&imported.meeting_id).unwrap();
        let names = |storage: &MeetingStorage| -> Vec<String> {
            storage
                .load_transcript(&id)
                .unwrap()
                .segments
                .iter()
                .map(|s| s.speaker_display())
                .collect()
        };
        assert_eq!(names(&storage), ["You", "Remote"]);
        storage.set_speaker_label(&id, "You", "PJ").unwrap();

        config.meeting.auto_label_from_title = true;
        let request = RetranscribeRequest {
            meeting: "latest".to_string(),
            keep_edits: true,
            ..Default::default()
        };
        retranscribe(&config, &request, counting(), &mut |_| {})
            .await
            .unwrap();
        assert_eq!(names(&storage), ["PJ", "Chris"]);
        let labels = storage.get_speaker_labels(&id).unwrap();
        assert_eq!(labels.get("Remote").map(String::as_str), Some("Chris"));
    }

    #[tokio::test]
    async fn a_meeting_without_audio_cannot_be_transcribed_again() {
        if !ffmpeg_present() {
            return;
        }
        let dir = tempfile::TempDir::new().unwrap();
        let config = config_in(&dir.path().join("meetings"));
        let storage =
            MeetingStorage::open(meeting_config_for(&config, None).storage.clone()).unwrap();
        let metadata = crate::meeting::MeetingMetadata::new(Some("Old".to_string()));
        storage.create_meeting(&metadata).unwrap();
        let err = retranscribe(
            &config,
            &RetranscribeRequest {
                meeting: "latest".to_string(),
                ..Default::default()
            },
            counting(),
            &mut |_| {},
        )
        .await
        .unwrap_err();
        assert!(
            err.to_string()
                .starts_with("This meeting was recorded without keeping audio"),
            "{err}"
        );
    }
}
