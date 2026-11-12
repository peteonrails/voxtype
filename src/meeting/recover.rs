//! Meetings the daemon never finished.
//!
//! A meeting is saved as it records: the transcript after every chunk, the
//! retained audio as it arrives. When the daemon crashes or is killed, the
//! folder keeps all of that, but nothing finishes the meeting: no end time,
//! no summary, no post-meeting command. On its next start the daemon marks
//! such meetings `interrupted` ([`MeetingStorage::interrupt_stale_meetings`])
//! and announces them on meeting.sock; `voxtype meeting recover` lists them
//! and finishes ([`finish`]) or discards ([`discard`]) each one.

use std::path::Path;

use chrono::{DateTime, Utc};
use serde::Serialize;

use super::data::{MeetingId, MeetingMetadata, MeetingStatus, Transcript};
use super::retained_audio;
use super::storage::{MeetingStorage, StorageError};

/// Trigger file, in the runtime dir: the recoverable meetings changed (one
/// was finished or discarded), so the daemon announces the list again.
pub const TRIGGER: &str = "meeting_recoverable";

/// A meeting that never finished, as `meeting recover --json` and the
/// `recoverable` event list it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RecoverableMeeting {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub started_at: DateTime<Utc>,
    /// Where the transcript ends on the meeting's timeline: the end of the
    /// last chunk transcribed (or of the last line, for a meeting recorded
    /// before 1.3.1).
    pub last_chunk_ms: u64,
    /// Whether retained audio is on disk.
    pub has_audio: bool,
    /// Chunks of retained audio past `last_chunk_ms`, which the daemon kept
    /// but never transcribed; `--finish` transcribes them.
    pub untranscribed_chunks: u32,
    /// Transcript lines on disk.
    pub lines: usize,
    /// When the meeting last wrote to its folder: about when the daemon
    /// stopped.
    pub stopped_at: DateTime<Utc>,
    /// From `started_at` to `stopped_at`.
    pub duration_secs: u64,
}

/// Why a meeting could not be finished or discarded.
#[derive(Debug, thiserror::Error)]
pub enum RecoverError {
    #[error("Meeting not found: {0}")]
    NotFound(String),
    #[error(
        "Meeting {id} is {status}, not interrupted. Only meetings the daemon never finished can be recovered: voxtype meeting recover"
    )]
    NotRecoverable { id: String, status: String },
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error(transparent)]
    Transcribe(#[from] crate::meeting::import::ImportError),
}

/// Whether `metadata` is a meeting nobody is finishing. `running` is the
/// meeting the daemon is recording now, if any: an active meeting is only
/// abandoned when it is not that one.
pub fn is_recoverable(metadata: &MeetingMetadata, running: Option<&MeetingId>) -> bool {
    match metadata.status {
        MeetingStatus::Interrupted => true,
        MeetingStatus::Active | MeetingStatus::Paused => running != Some(&metadata.id),
        MeetingStatus::Completed | MeetingStatus::Cancelled => false,
    }
}

/// Every meeting that never finished, newest first. `chunk_secs` is
/// `[meeting] chunk_duration_secs`, for counting untranscribed chunks.
pub fn recoverable(
    storage: &MeetingStorage,
    running: Option<&MeetingId>,
    chunk_secs: u32,
) -> Result<Vec<RecoverableMeeting>, StorageError> {
    Ok(storage
        .list_meetings(None)?
        .into_iter()
        .filter(|m| is_recoverable(m, running))
        .map(|m| describe(storage, &m, chunk_secs))
        .collect())
}

fn describe(
    storage: &MeetingStorage,
    metadata: &MeetingMetadata,
    chunk_secs: u32,
) -> RecoverableMeeting {
    let transcript = storage.load_transcript(&metadata.id).unwrap_or_default();
    let dir = metadata.storage_path.as_deref();
    let stopped_at = stopped_at(dir, metadata, &transcript);
    let last_chunk_ms = last_chunk_ms(&transcript);
    let audio_ms = dir.and_then(audio_ms);
    RecoverableMeeting {
        id: metadata.id.to_string(),
        title: metadata.title.clone(),
        started_at: metadata.started_at,
        last_chunk_ms,
        has_audio: dir.is_some_and(|d| !retained_audio::retained_files(d).is_empty()),
        untranscribed_chunks: audio_ms
            .map(|audio| untranscribed_chunks(audio, last_chunk_ms, chunk_secs))
            .unwrap_or(0),
        lines: transcript
            .segments
            .iter()
            .filter(|s| !s.text.trim().is_empty())
            .count(),
        stopped_at,
        duration_secs: (stopped_at - metadata.started_at).num_seconds().max(0) as u64,
    }
}

/// Where the transcript ends on the meeting's timeline.
pub fn last_chunk_ms(transcript: &Transcript) -> u64 {
    if transcript.audio_ms > 0 {
        return transcript.audio_ms;
    }
    transcript
        .segments
        .iter()
        .map(|s| s.end_ms)
        .max()
        .unwrap_or(0)
}

/// Less than this past the transcript is the end of a chunk the daemon did
/// transcribe, not audio it missed.
const MIN_UNTRANSCRIBED_MS: u64 = 1_000;

/// Chunks of `audio_ms` of retained audio that lie past `last_chunk_ms`.
pub fn untranscribed_chunks(audio_ms: u64, last_chunk_ms: u64, chunk_secs: u32) -> u32 {
    // The same slack at the far end: an encoder pads the last page a
    // little, so 30.02 s is one 30 s chunk, not two.
    let left = audio_ms.saturating_sub(last_chunk_ms);
    if left < MIN_UNTRANSCRIBED_MS {
        return 0;
    }
    (left - MIN_UNTRANSCRIBED_MS)
        .div_ceil(chunk_secs.max(1) as u64 * 1000)
        .max(1) as u32
}

/// How long the retained audio is: the mic track's (or the mixed track's)
/// length, read from a WAV's size or with ffprobe. `None` without audio,
/// or when it cannot be read.
pub fn audio_ms(dir: &Path) -> Option<u64> {
    let files = retained_audio::retained_files(dir);
    let (_, file) = files
        .iter()
        .find(|(stem, _)| *stem == retained_audio::MIC)
        .or_else(|| files.first())?;
    let path = dir.join(file);
    if file.ends_with(".wav") {
        // 16 kHz mono 16-bit PCM behind a 44-byte header.
        let bytes = std::fs::metadata(&path).ok()?.len().saturating_sub(44);
        return Some(bytes / 2 * 1000 / retained_audio::SAMPLE_RATE as u64);
    }
    let ff = crate::meeting::import::Ffmpeg::detect().ok()?;
    crate::meeting::import::probe(&ff, &path)
        .ok()
        .map(|p| p.duration_ms)
}

/// When the meeting stopped recording: the last write to its audio, which
/// the daemon writes as it records and nothing else touches; without audio
/// the last write to its transcript, or the end of its last line.
fn stopped_at(
    dir: Option<&Path>,
    metadata: &MeetingMetadata,
    transcript: &Transcript,
) -> DateTime<Utc> {
    let modified = |dir: &Path, files: Vec<String>| {
        files
            .iter()
            .filter_map(|f| std::fs::metadata(dir.join(f)).ok()?.modified().ok())
            .max()
            .map(DateTime::<Utc>::from)
    };
    let written = dir.and_then(|dir| {
        let audio: Vec<String> = retained_audio::retained_files(dir)
            .into_iter()
            .map(|(_, f)| f)
            .collect();
        if audio.is_empty() {
            modified(dir, vec!["transcript.json".to_string()])
        } else {
            modified(dir, audio)
        }
    });
    let last_line = transcript
        .segments
        .iter()
        .map(|s| s.end_ms)
        .max()
        .map(|ms| metadata.started_at + chrono::Duration::milliseconds(ms as i64));
    written
        .or(last_line)
        .unwrap_or(metadata.started_at)
        .max(metadata.started_at)
}

fn load_recoverable(
    storage: &MeetingStorage,
    id: &MeetingId,
    running: Option<&MeetingId>,
) -> Result<MeetingMetadata, RecoverError> {
    let metadata = storage
        .get_meeting(id)?
        .ok_or_else(|| RecoverError::NotFound(id.to_string()))?;
    if !is_recoverable(&metadata, running) {
        return Err(RecoverError::NotRecoverable {
            id: id.to_string(),
            status: crate::meeting::storage::status_to_string(metadata.status).to_string(),
        });
    }
    Ok(metadata)
}

/// Finish a meeting from what is on disk, as stopping it would have: the
/// end time (when it last wrote), the transcript and its markdown, whether
/// audio was kept, and the status `completed`. The summary and the
/// post-meeting command are the caller's, as after a normal stop.
pub fn finish(
    storage: &MeetingStorage,
    id: &MeetingId,
    running: Option<&MeetingId>,
) -> Result<MeetingMetadata, RecoverError> {
    let mut metadata = load_recoverable(storage, id, running)?;
    let transcript = match storage.load_transcript_all(id) {
        Ok(t) => Some(t),
        Err(StorageError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.into()),
    };
    let ended = stopped_at(
        metadata.storage_path.as_deref(),
        &metadata,
        transcript.as_ref().unwrap_or(&Transcript::default()),
    );
    metadata.status = MeetingStatus::Completed;
    metadata.ended_at = Some(ended);
    metadata.duration_secs = Some((ended - metadata.started_at).num_seconds().max(0) as u64);
    if let Some(ref t) = transcript {
        metadata.chunk_count = t.total_chunks;
    }
    if let Some(ref dir) = metadata.storage_path {
        metadata.audio_retained = !retained_audio::retained_files(dir).is_empty();
    }
    storage.update_meeting(&metadata)?;
    if let Some(ref t) = transcript {
        // Rewrites transcript.md with the final metadata.
        storage.save_transcript(id, t)?;
    }
    Ok(metadata)
}

/// Transcribe the retained audio past the transcript's end: what the daemon
/// kept but did not get to before it stopped. Uses the meeting's own
/// engine, model and diarization, as `meeting.json` records them, and adds
/// the new lines to the transcript after the old ones. `resident` is a
/// transcriber to use instead of loading one (tests pass a fake).
/// Returns how many chunks were transcribed (0 when nothing was left).
pub async fn transcribe_tail(
    config: &crate::config::Config,
    storage: &MeetingStorage,
    id: &MeetingId,
    running: Option<&MeetingId>,
    resident: Option<std::sync::Arc<dyn crate::transcribe::Transcriber>>,
) -> Result<u32, RecoverError> {
    use crate::meeting::import::{self, daemon_for, meeting_config_for, Channels, Ffmpeg};
    let metadata = load_recoverable(storage, id, running)?;
    let Some(dir) = metadata.storage_path.clone() else {
        return Ok(0);
    };
    if retained_audio::retained_files(&dir).is_empty() {
        return Ok(0);
    }
    let mut transcript = match storage.load_transcript_all(id) {
        Ok(t) => t,
        Err(StorageError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
            Transcript::default()
        }
        Err(e) => return Err(e.into()),
    };
    let from = last_chunk_ms(&transcript);
    let ff = Ffmpeg::detect()?;
    let mut source = crate::meeting::retranscribe::retained_source(&ff, &dir)?;
    let chunk_secs = config.meeting.chunk_duration_secs;
    if untranscribed_chunks(source.duration_ms, from, chunk_secs) == 0 {
        return Ok(0);
    }
    source.start_ms = from;

    let context = storage.get_context(id)?;
    let options = crate::config::MeetingStartOptions {
        engine: context.engine.clone(),
        model: context.model.clone(),
        ..Default::default()
    };
    // The engine the meeting recorded with, or the configured one when
    // this build cannot run that (or it was never recorded).
    let effective = crate::meeting::start::meeting_transcription_config(config, &options)
        .or_else(|e| {
            tracing::warn!("Transcribing with the configured engine instead: {}", e);
            crate::meeting::start::meeting_transcription_config(config, &Default::default())
        })
        .map_err(import::ImportError::Config)?;
    let mut meeting_config = meeting_config_for(config, context.diarization.as_deref());
    if context.diarization.is_none() {
        meeting_config.diarization = None;
    }
    // Transcribed into a scratch store, then added to the meeting.
    let scratch = tempfile::TempDir::new().map_err(StorageError::Io)?;
    meeting_config.storage.storage_path = scratch.path().to_path_buf();
    if source.channels == Channels::Mono {
        import::mono_diarization(&mut meeting_config);
    }
    let vad_threshold = meeting_config.vad_threshold;
    let scratch_storage = MeetingStorage::open(meeting_config.storage.clone())?;
    let mut daemon = daemon_for(meeting_config, &effective, resident)?;
    let scratch_id = daemon
        .start_at(metadata.title.clone(), None, Some(metadata.started_at))
        .await
        .map_err(import::ImportError::from)?;
    let mut chunks = 0;
    let result = import::transcribe_source(
        &mut daemon,
        &ff,
        &source,
        chunk_secs,
        vad_threshold,
        &mut |done, _| chunks = done,
    )
    .await;
    let stopped = daemon.stop().await;
    result?;
    stopped.map_err(import::ImportError::from)?;
    let tail = match scratch_storage.load_transcript_all(&scratch_id) {
        Ok(t) => t,
        Err(StorageError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
            Transcript::default()
        }
        Err(e) => return Err(e.into()),
    };

    let first_id = transcript
        .segments
        .iter()
        .map(|s| s.id + 1)
        .max()
        .unwrap_or(0);
    let chunk_base = transcript.total_chunks;
    for (id, mut seg) in (first_id..).zip(tail.segments) {
        seg.id = id;
        seg.start_ms += from;
        seg.end_ms += from;
        seg.chunk_id += chunk_base;
        transcript.add_segment(seg);
    }
    transcript.total_chunks += tail.total_chunks;
    transcript.audio_ms = source.duration_ms.max(from);
    storage.save_transcript(id, &transcript)?;
    Ok(chunks)
}

/// Delete a meeting that never finished, folder and all.
pub fn discard(
    storage: &MeetingStorage,
    id: &MeetingId,
    running: Option<&MeetingId>,
) -> Result<(), RecoverError> {
    load_recoverable(storage, id, running)?;
    storage.delete_meeting(id)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meeting::data::{AudioSource, TranscriptSegment};
    use crate::meeting::StorageConfig;

    fn storage(dir: &Path) -> MeetingStorage {
        MeetingStorage::open(StorageConfig {
            storage_path: dir.to_path_buf(),
            retain_audio: false,
            max_meetings: 0,
            my_name: None,
        })
        .unwrap()
    }

    /// A meeting as a killed daemon leaves it: active, with a transcript.
    fn abandoned(storage: &MeetingStorage, title: &str) -> MeetingMetadata {
        let mut metadata = MeetingMetadata::new(Some(title.to_string()));
        metadata.started_at = Utc::now() - chrono::Duration::minutes(10);
        let dir = storage.create_meeting(&metadata).unwrap();
        metadata.storage_path = Some(dir);
        let mut transcript = Transcript::new();
        for (id, text) in [(0, "Shall we start?"), (1, "Yes.")] {
            let mut seg = TranscriptSegment::new(
                id,
                id as u64 * 5_000,
                id as u64 * 5_000 + 4_000,
                text.into(),
                0,
            );
            seg.source = AudioSource::Microphone;
            transcript.add_segment(seg);
        }
        transcript.total_chunks = 1;
        storage.save_transcript(&metadata.id, &transcript).unwrap();
        metadata
    }

    #[test]
    fn untranscribed_audio_counts_in_whole_chunks_with_slack() {
        assert_eq!(untranscribed_chunks(30_400, 30_000, 30), 0);
        assert_eq!(untranscribed_chunks(31_500, 30_000, 30), 1);
        assert_eq!(untranscribed_chunks(60_020, 30_000, 30), 1);
        assert_eq!(untranscribed_chunks(61_500, 30_000, 30), 2);
        assert_eq!(untranscribed_chunks(10_000, 30_000, 30), 0);
    }

    #[test]
    fn only_meetings_nobody_is_finishing_are_recoverable() {
        let dir = tempfile::tempdir().unwrap();
        let s = storage(dir.path());
        let crashed = abandoned(&s, "Crashed");
        let running = abandoned(&s, "Running");
        let mut done = abandoned(&s, "Done");
        done.complete();
        s.update_meeting(&done).unwrap();

        let ids: Vec<String> = recoverable(&s, Some(&running.id), 30)
            .unwrap()
            .into_iter()
            .map(|m| m.id)
            .collect();
        assert_eq!(ids, vec![crashed.id.to_string()]);

        // On the next start the daemon marks both left-over meetings.
        assert_eq!(s.interrupt_stale_meetings().unwrap(), 2);
        let listed = recoverable(&s, None, 30).unwrap();
        assert_eq!(listed.len(), 2);
        let crashed = listed
            .iter()
            .find(|m| m.title.as_deref() == Some("Crashed"))
            .unwrap();
        assert_eq!(crashed.lines, 2);
        assert!(!crashed.has_audio);
        assert_eq!(crashed.untranscribed_chunks, 0);
        assert_eq!(crashed.last_chunk_ms, 9_000);
        assert!(crashed.stopped_at >= crashed.started_at);
    }

    #[test]
    fn finishing_completes_the_meeting_from_its_folder() {
        let dir = tempfile::tempdir().unwrap();
        let s = storage(dir.path());
        let m = abandoned(&s, "Crashed");
        s.interrupt_stale_meetings().unwrap();
        let folder = m.storage_path.clone().unwrap();
        assert_eq!(
            crate::meeting::manifest::Manifest::read(&folder)
                .unwrap()
                .status,
            "interrupted"
        );

        let done = finish(&s, &m.id, None).unwrap();
        assert_eq!(done.status, MeetingStatus::Completed);
        assert!(done.ended_at.unwrap() >= m.started_at);
        assert_eq!(done.chunk_count, 1);
        let saved = s.get_meeting(&m.id).unwrap().unwrap();
        assert_eq!(saved.status, MeetingStatus::Completed);
        assert!(saved.duration_secs.is_some());
        assert_eq!(
            crate::meeting::manifest::Manifest::read(&folder)
                .unwrap()
                .status,
            "completed"
        );
        assert!(std::fs::read_to_string(folder.join("transcript.md"))
            .unwrap()
            .contains("Shall we start?"));

        // Once finished it is an ordinary meeting.
        assert!(recoverable(&s, None, 30).unwrap().is_empty());
        assert!(matches!(
            finish(&s, &m.id, None),
            Err(RecoverError::NotRecoverable { .. })
        ));
        assert!(matches!(
            discard(&s, &m.id, None),
            Err(RecoverError::NotRecoverable { .. })
        ));
    }

    #[test]
    fn the_running_meeting_is_never_finished_or_discarded() {
        let dir = tempfile::tempdir().unwrap();
        let s = storage(dir.path());
        let m = abandoned(&s, "Running");
        assert!(matches!(
            discard(&s, &m.id, Some(&m.id)),
            Err(RecoverError::NotRecoverable { .. })
        ));
        assert!(s.get_meeting(&m.id).unwrap().is_some());
    }

    #[test]
    fn discarding_deletes_the_meeting_and_its_folder() {
        let dir = tempfile::tempdir().unwrap();
        let s = storage(dir.path());
        let m = abandoned(&s, "Crashed");
        let folder = m.storage_path.clone().unwrap();
        discard(&s, &m.id, None).unwrap();
        assert!(s.get_meeting(&m.id).unwrap().is_none());
        assert!(!folder.exists());
    }

    #[test]
    fn a_meeting_killed_before_its_first_chunk_finishes_empty() {
        let dir = tempfile::tempdir().unwrap();
        let s = storage(dir.path());
        let metadata = MeetingMetadata::new(Some("Empty".to_string()));
        s.create_meeting(&metadata).unwrap();
        let listed = recoverable(&s, None, 30).unwrap();
        assert_eq!(listed[0].lines, 0);
        let done = finish(&s, &metadata.id, None).unwrap();
        assert_eq!(done.status, MeetingStatus::Completed);
    }
}
