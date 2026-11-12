//! `meeting.json`: one manifest per meeting folder, describing the meeting
//! and the files beside it so the folder reads without voxtype.
//!
//! This is a stable contract (documented in docs/INTEGRATIONS.md): fields
//! are only ever added, every documented field is always present (`null`
//! when unknown), and `manifest_version` changes only if a field's meaning
//! does.

use crate::meeting::assistant::summarize::SUMMARY_FILE;
use crate::meeting::data::MeetingMetadata;
use crate::meeting::retained_audio;
use crate::meeting::storage::{NOTES_FILE, TRANSCRIPT_MARKDOWN_FILE};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// The manifest's file name in the meeting folder.
pub const MANIFEST_FILE: &str = "meeting.json";

/// Bumped only if an existing field changes meaning.
pub const MANIFEST_VERSION: u32 = 1;

/// The transcript as JSON.
pub const TRANSCRIPT_JSON_FILE: &str = "transcript.json";

/// What a meeting was recorded with, fixed when it starts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MeetingContext {
    /// Transcription engine, e.g. `parakeet`
    pub engine: Option<String>,
    /// The engine's model
    pub model: Option<String>,
    /// Diarization backend, `None` when diarization was off
    pub diarization: Option<String>,
    /// The voxtype version that recorded the meeting
    pub voxtype_version: Option<String>,
    /// The echo handling the meeting used: `aec`, `gtcrn` or `off`
    pub echo_cancel: Option<String>,
}

/// A speaker in the transcript and the name it is shown with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestSpeaker {
    /// The speaker as the diarizer named it (`You`, `Remote`, `SPEAKER_00`)
    pub id: String,
    /// The name given with `meeting label`, or `id` when it has none
    pub name: String,
}

/// A file in the meeting folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestFile {
    /// File name, relative to the meeting folder
    pub name: String,
    /// What the file holds
    pub kind: String,
    /// Size in bytes when the manifest was written
    pub bytes: u64,
}

/// The contents of `meeting.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub manifest_version: u32,
    pub id: String,
    pub title: Option<String>,
    pub url: Option<String>,
    /// `active`, `paused`, `completed` or `cancelled`
    pub status: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub duration_secs: Option<u64>,
    pub engine: Option<String>,
    pub model: Option<String>,
    pub diarization: Option<String>,
    /// `aec`, `gtcrn` or `off`; null for meetings recorded before 1.3.0
    #[serde(default)]
    pub echo_cancel: Option<String>,
    pub speakers: Vec<ManifestSpeaker>,
    pub audio_retained: bool,
    pub files: Vec<ManifestFile>,
    pub voxtype_version: String,
}

impl Manifest {
    /// Describe a meeting and the files in its folder `dir`.
    pub fn build(
        dir: &Path,
        metadata: &MeetingMetadata,
        context: &MeetingContext,
        speaker_keys: &[String],
        labels: &HashMap<String, String>,
    ) -> Self {
        let files = folder_files(dir);
        let audio_retained =
            metadata.audio_retained || files.iter().any(|f| f.kind.starts_with("audio_"));
        Self {
            manifest_version: MANIFEST_VERSION,
            id: metadata.id.to_string(),
            title: metadata.title.clone(),
            url: metadata.url.clone(),
            status: serde_json::to_value(metadata.status)
                .ok()
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_default(),
            started_at: metadata.started_at,
            ended_at: metadata.ended_at,
            duration_secs: metadata.duration_secs,
            engine: context.engine.clone().or_else(|| metadata.model.clone()),
            model: context.model.clone(),
            diarization: context.diarization.clone(),
            echo_cancel: context.echo_cancel.clone(),
            speakers: speaker_keys
                .iter()
                .map(|id| ManifestSpeaker {
                    id: id.clone(),
                    name: labels.get(id).cloned().unwrap_or_else(|| id.clone()),
                })
                .collect(),
            audio_retained,
            files,
            voxtype_version: context
                .voxtype_version
                .clone()
                .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string()),
        }
    }

    /// Write the manifest into `dir`, atomically.
    pub fn write(&self, dir: &Path) -> std::io::Result<()> {
        let mut json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        json.push('\n');
        let staged = dir.join(format!(".{}.{}", MANIFEST_FILE, std::process::id()));
        std::fs::write(&staged, json)?;
        std::fs::rename(&staged, dir.join(MANIFEST_FILE))
    }

    /// Read the manifest in `dir`.
    pub fn read(dir: &Path) -> std::io::Result<Self> {
        let json = std::fs::read_to_string(dir.join(MANIFEST_FILE))?;
        serde_json::from_str(&json).map_err(std::io::Error::other)
    }
}

/// The meeting's files present in `dir`, in a fixed order.
fn folder_files(dir: &Path) -> Vec<ManifestFile> {
    let mut candidates: Vec<(String, String)> = [
        (TRANSCRIPT_JSON_FILE, "transcript"),
        (TRANSCRIPT_MARKDOWN_FILE, "transcript_markdown"),
        (NOTES_FILE, "notes"),
        (SUMMARY_FILE, "summary"),
    ]
    .into_iter()
    .map(|(name, kind)| (name.to_string(), kind.to_string()))
    .collect();
    candidates.extend(
        retained_audio::retained_files(dir)
            .into_iter()
            .map(|(stem, name)| (name, format!("audio_{stem}"))),
    );
    candidates
        .into_iter()
        .filter_map(|(name, kind)| {
            let meta = std::fs::metadata(dir.join(&name)).ok()?;
            meta.is_file().then_some(ManifestFile {
                name,
                kind,
                bytes: meta.len(),
            })
        })
        .collect()
}
