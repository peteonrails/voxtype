//! `voxtype meeting import`: turn a recording made elsewhere into a normal
//! meeting, with the same storage, transcript, manifest, retained audio,
//! summary and post-meeting hook as one recorded live.
//!
//! The file is decoded by ffmpeg (so anything ffmpeg reads works) and fed
//! through a [`MeetingDaemon`] chunk by chunk, exactly as live audio is, so
//! diarization, echo dedup and segment numbering behave the same. A mono
//! file, or a multichannel one downmixed with `--channels mono`, is one
//! track with no side; `--channels mic,loopback` maps a stereo call
//! recording's channels to You and Remote so `simple` and `hybrid`
//! diarization can tell the sides apart.
//!
//! The module also holds what `meeting retranscribe` shares with import:
//! the decoder, the chunk driver and progress reporting.

use crate::config::{Config, MeetingStartOptions};
use crate::meeting::{
    retained_audio, AudioSource, MeetingConfig, MeetingDaemon, MeetingEvent, MeetingId,
    MeetingStorage, StorageConfig, StorageError,
};
use crate::transcribe::Transcriber;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;

/// Meeting audio runs at 16 kHz.
const SAMPLE_RATE: u32 = 16_000;

/// The last import or retranscribe progress, for the daemon to relay.
pub const PROGRESS_FILE: &str = "meeting_import.json";
/// The daemon trigger that announces progress on meeting.sock.
pub const PROGRESS_TRIGGER: &str = "meeting_import";

/// Why an import or retranscribe failed.
#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error(
        "Importing and transcribing again need ffmpeg and ffprobe. Install ffmpeg \
         (sudo pacman -S ffmpeg, sudo apt install ffmpeg, sudo dnf install ffmpeg-free)."
    )]
    MissingFfmpeg,

    #[error("Cannot read {path}: {reason}")]
    Unreadable { path: String, reason: String },

    #[error("Invalid --channels '{spec}': {reason}")]
    BadChannels { spec: String, reason: String },

    #[error(
        "This meeting was recorded without keeping audio, so there is nothing to \
         transcribe again. Turn on meeting.retain_audio for future meetings."
    )]
    NoAudio,

    #[error("{0}")]
    Config(String),

    #[error("Decoding the audio failed: {0}")]
    Decode(String),

    #[error(transparent)]
    Storage(#[from] StorageError),

    #[error(transparent)]
    Meeting(#[from] crate::error::VoxtypeError),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// ffmpeg and ffprobe.
#[derive(Debug, Clone)]
pub struct Ffmpeg {
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
}

impl Ffmpeg {
    /// Both on PATH, or [`ImportError::MissingFfmpeg`].
    pub fn detect() -> Result<Self, ImportError> {
        match (which::which("ffmpeg"), which::which("ffprobe")) {
            (Ok(ffmpeg), Ok(ffprobe)) => Ok(Self { ffmpeg, ffprobe }),
            _ => Err(ImportError::MissingFfmpeg),
        }
    }
}

/// What ffprobe says about a file's first audio stream.
#[derive(Debug, Clone, PartialEq)]
pub struct Probe {
    pub channels: u32,
    pub duration_ms: u64,
    /// The `creation_time` tag, when the file has one
    pub created: Option<DateTime<Utc>>,
}

/// Probe `file`'s first audio stream.
pub fn probe(ff: &Ffmpeg, file: &Path) -> Result<Probe, ImportError> {
    let unreadable = |reason: String| ImportError::Unreadable {
        path: file.display().to_string(),
        reason,
    };
    let out = Command::new(&ff.ffprobe)
        .args(["-v", "error", "-select_streams", "a:0"])
        .args([
            "-show_entries",
            "stream=channels:format=duration:format_tags=creation_time",
        ])
        .args(["-of", "json"])
        .arg(file)
        .stdin(Stdio::null())
        .output()?;
    if !out.status.success() {
        return Err(unreadable(
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ));
    }
    let json: serde_json::Value =
        serde_json::from_slice(&out.stdout).map_err(|e| unreadable(e.to_string()))?;
    let channels = json["streams"][0]["channels"]
        .as_u64()
        .ok_or_else(|| unreadable("it has no audio stream".to_string()))? as u32;
    let duration_ms = json["format"]["duration"]
        .as_str()
        .and_then(|d| d.parse::<f64>().ok())
        .map(|secs| (secs * 1000.0).round() as u64)
        .unwrap_or(0);
    let created = json["format"]["tags"]["creation_time"]
        .as_str()
        .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
        .map(|t| t.with_timezone(&Utc));
    Ok(Probe {
        channels,
        duration_ms,
        created,
    })
}

/// Which side of the call one channel holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// You (the microphone)
    Mic,
    /// The others (the loopback)
    Loopback,
    /// Not used
    Ignore,
}

/// How a file's channels become meeting tracks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Channels {
    /// Every channel mixed into one track with no side
    Mono,
    /// One entry per channel, in order
    Mapped(Vec<Side>),
}

impl Channels {
    /// Parse `--channels`: `mono`, or one of `mic`/`you`, `loopback`/`remote`
    /// or `ignore`/`-` per channel, comma-separated.
    pub fn parse(spec: &str) -> Result<Self, ImportError> {
        let bad = |reason: &str| ImportError::BadChannels {
            spec: spec.to_string(),
            reason: reason.to_string(),
        };
        if spec.trim().eq_ignore_ascii_case("mono") {
            return Ok(Channels::Mono);
        }
        let sides = spec
            .split(',')
            .map(|s| match s.trim().to_ascii_lowercase().as_str() {
                "mic" | "you" => Ok(Side::Mic),
                "loopback" | "remote" => Ok(Side::Loopback),
                "ignore" | "-" => Ok(Side::Ignore),
                _ => Err(bad(&format!(
                    "'{}' is not mic, loopback or ignore (use 'mono' to mix every channel)",
                    s.trim()
                ))),
            })
            .collect::<Result<Vec<_>, _>>()?;
        if !sides.iter().any(|s| *s != Side::Ignore) {
            return Err(bad("it ignores every channel"));
        }
        Ok(Channels::Mapped(sides))
    }

    /// Check the map fits a file with `channels` channels.
    pub fn check(&self, channels: u32, spec: &str) -> Result<(), ImportError> {
        match self {
            Channels::Mapped(sides) if sides.len() != channels as usize => {
                Err(ImportError::BadChannels {
                    spec: spec.to_string(),
                    reason: format!(
                        "the file has {channels} channel{}, so name {channels} \
                         (for example {})",
                        if channels == 1 { "" } else { "s" },
                        example_map(channels)
                    ),
                })
            }
            _ => Ok(()),
        }
    }

    /// The map as `--channels` takes it.
    pub fn describe(&self) -> String {
        match self {
            Channels::Mono => "mono".to_string(),
            Channels::Mapped(sides) => sides
                .iter()
                .map(|s| match s {
                    Side::Mic => "mic",
                    Side::Loopback => "loopback",
                    Side::Ignore => "ignore",
                })
                .collect::<Vec<_>>()
                .join(","),
        }
    }

    fn channels_of(&self, side: Side) -> Vec<usize> {
        match self {
            Channels::Mono => Vec::new(),
            Channels::Mapped(sides) => sides
                .iter()
                .enumerate()
                .filter(|(_, s)| **s == side)
                .map(|(i, _)| i)
                .collect(),
        }
    }

    /// Channels decoded per frame.
    fn width(&self) -> usize {
        match self {
            Channels::Mono => 1,
            Channels::Mapped(sides) => sides.len(),
        }
    }
}

fn example_map(channels: u32) -> String {
    match channels {
        1 => "mono".to_string(),
        2 => "mic,loopback".to_string(),
        n => std::iter::once("mic")
            .chain(std::iter::repeat_n("loopback", n as usize - 1))
            .collect::<Vec<_>>()
            .join(","),
    }
}

/// Audio to transcribe: one file, or one file per channel (as a meeting's
/// retained mic and loopback tracks are), and how its channels map.
#[derive(Debug, Clone)]
pub struct Source {
    pub inputs: Vec<PathBuf>,
    pub channels: Channels,
    pub duration_ms: u64,
}

impl Source {
    fn decode_command(&self, ff: &Ffmpeg) -> Command {
        let mut cmd = Command::new(&ff.ffmpeg);
        cmd.args(["-hide_banner", "-loglevel", "error", "-nostdin"]);
        for input in &self.inputs {
            cmd.arg("-i").arg(input);
        }
        if self.inputs.len() > 1 {
            let labels: String = (0..self.inputs.len()).map(|i| format!("[{i}:a]")).collect();
            cmd.args([
                "-filter_complex",
                &format!("{labels}amerge=inputs={}[a]", self.inputs.len()),
                "-map",
                "[a]",
            ]);
        } else {
            cmd.args(["-map", "0:a:0"]);
        }
        cmd.args(["-f", "s16le", "-ar", &SAMPLE_RATE.to_string()])
            .args(["-ac", &self.channels.width().to_string(), "pipe:1"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        cmd
    }

    /// Estimated chunks for progress.
    fn chunks_total(&self, chunk_secs: u32) -> u32 {
        (self.duration_ms.div_ceil(chunk_secs.max(1) as u64 * 1000)).max(1) as u32
    }
}

/// How far an import or retranscribe has got.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Progress {
    pub meeting_id: String,
    /// `import` or `retranscribe`
    pub operation: String,
    /// `transcribing`, `done` or `failed`
    pub state: String,
    pub chunks_done: u32,
    /// An estimate from the duration until the end, when it is exact
    pub chunks_total: u32,
    pub position_ms: u64,
    pub duration_ms: u64,
}

/// Record `progress` for the daemon and ask it to announce it on
/// meeting.sock. Without a daemon nothing reads it.
pub fn announce(runtime_dir: &Path, progress: &Progress) {
    let write = || -> std::io::Result<()> {
        std::fs::create_dir_all(runtime_dir)?;
        let staged = runtime_dir.join(format!(".{PROGRESS_FILE}.{}", std::process::id()));
        std::fs::write(
            &staged,
            serde_json::to_string(progress).map_err(std::io::Error::other)?,
        )?;
        std::fs::rename(&staged, runtime_dir.join(PROGRESS_FILE))?;
        std::fs::write(runtime_dir.join(PROGRESS_TRIGGER), "")
    };
    if let Err(e) = write() {
        tracing::debug!("Could not record import progress: {}", e);
    }
}

/// The last progress [`announce`]d.
pub fn read_progress(runtime_dir: &Path) -> Option<Progress> {
    let json = std::fs::read_to_string(runtime_dir.join(PROGRESS_FILE)).ok()?;
    serde_json::from_str(&json).ok()
}

/// Decode `source` and feed it through `daemon` chunk by chunk, as the live
/// meeting path does: chunks of `chunk_secs`, cut in a pause both tracks
/// share. Calls `on_chunk(chunks_done, position_ms)` after each chunk.
/// Returns the milliseconds of audio decoded.
pub async fn transcribe_source(
    daemon: &mut MeetingDaemon,
    ff: &Ffmpeg,
    source: &Source,
    chunk_secs: u32,
    vad_threshold: f32,
    on_chunk: &mut dyn FnMut(u32, u64),
) -> Result<u64, ImportError> {
    let mic_channels = source.channels.channels_of(Side::Mic);
    let loopback_channels = source.channels.channels_of(Side::Loopback);
    let mono = source.channels == Channels::Mono;
    let width = source.channels.width();
    let chunk_samples = SAMPLE_RATE as usize * chunk_secs.max(1) as usize;

    let mut child = source.decode_command(ff).spawn()?;
    let mut stdout = child.stdout.take().expect("piped");
    let mut stderr = child.stderr.take().expect("piped");
    let errors = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stderr.read_to_string(&mut text);
        text
    });

    // In mono both live in `a`. Mapped, `a` is the mic and `b` the
    // loopback, and the primary track for cutting is whichever exists.
    let (mut a, mut b) = (Vec::<f32>::new(), Vec::<f32>::new());
    let mic_primary = mono || !mic_channels.is_empty();
    let mut pending = Vec::<u8>::new();
    let mut block = vec![0u8; SAMPLE_RATE as usize * 2 * width];
    let (mut chunks, mut decoded) = (0u32, 0usize);

    loop {
        let n = stdout.read(&mut block)?;
        let done = n == 0;
        pending.extend_from_slice(&block[..n]);
        let frame_bytes = 2 * width;
        let frames = pending.len() / frame_bytes;
        for frame in pending[..frames * frame_bytes].chunks_exact(frame_bytes) {
            let sample =
                |ch: usize| i16::from_le_bytes([frame[2 * ch], frame[2 * ch + 1]]) as f32 / 32768.0;
            if mono {
                a.push(sample(0));
            } else {
                let sum =
                    |chs: &[usize]| chs.iter().map(|&c| sample(c)).sum::<f32>().clamp(-1.0, 1.0);
                if !mic_channels.is_empty() {
                    a.push(sum(&mic_channels));
                }
                if !loopback_channels.is_empty() {
                    b.push(sum(&loopback_channels));
                }
            }
        }
        decoded += frames;
        pending.drain(..frames * frame_bytes);

        loop {
            let (primary, other) = if mic_primary { (&a, &b) } else { (&b, &a) };
            let ready = primary.len() >= chunk_samples;
            if !(ready || (done && !primary.is_empty())) {
                break;
            }
            let cut = if ready {
                crate::meeting::chunk::chunk_cut_point(
                    primary,
                    other,
                    chunk_samples,
                    SAMPLE_RATE,
                    vad_threshold,
                )
            } else {
                primary.len()
            };
            let a_chunk: Vec<f32> = a.drain(..cut.min(a.len())).collect();
            let b_chunk: Vec<f32> = b.drain(..cut.min(b.len())).collect();
            if mono {
                daemon
                    .process_chunk_with_source(a_chunk, AudioSource::Unknown)
                    .await?;
            } else {
                daemon.process_pair(a_chunk, b_chunk).await;
            }
            chunks += 1;
            let position = decoded.saturating_sub(a.len().max(b.len()));
            on_chunk(chunks, position as u64 * 1000 / SAMPLE_RATE as u64);
        }
        if done {
            break;
        }
    }

    let status = child.wait()?;
    let errors = errors.join().unwrap_or_default();
    if !status.success() {
        return Err(ImportError::Decode(errors.trim().to_string()));
    }
    Ok(decoded as u64 * 1000 / SAMPLE_RATE as u64)
}

/// Write the meeting's audio into `dir` from `source`: `mixed` always, and
/// `mic` and `loopback` when channels map to them. Opus when ffmpeg has
/// libopus, WAV otherwise. Returns the files written.
pub fn keep_audio(ff: &Ffmpeg, source: &Source, dir: &Path) -> Result<Vec<String>, ImportError> {
    let format = match retained_audio::Encoder::detect() {
        retained_audio::Encoder::Ffmpeg(_) => retained_audio::AudioFormat::Opus,
        retained_audio::Encoder::Wav => retained_audio::AudioFormat::Wav,
    };
    let mut cmd = Command::new(&ff.ffmpeg);
    cmd.args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y"]);
    for input in &source.inputs {
        cmd.arg("-i").arg(input);
    }

    // One mono output per track, each a sum of the channels it takes.
    let merged = if source.inputs.len() > 1 {
        let labels: String = (0..source.inputs.len())
            .map(|i| format!("[{i}:a]"))
            .collect();
        format!("{labels}amerge=inputs={}", source.inputs.len())
    } else {
        "[0:a:0]anull".to_string()
    };
    let mut tracks: Vec<(&str, Vec<usize>)> = Vec::new();
    match &source.channels {
        Channels::Mono => tracks.push((retained_audio::MIXED, Vec::new())),
        Channels::Mapped(_) => {
            let mic = source.channels.channels_of(Side::Mic);
            let loopback = source.channels.channels_of(Side::Loopback);
            let both: Vec<usize> = mic.iter().chain(&loopback).copied().collect();
            if !mic.is_empty() {
                tracks.push((retained_audio::MIC, mic));
            }
            if !loopback.is_empty() {
                tracks.push((retained_audio::LOOPBACK, loopback));
            }
            tracks.push((retained_audio::MIXED, both));
        }
    }
    let pan = |chs: &[usize]| {
        if chs.is_empty() {
            // Every channel, averaged, as ffmpeg's own downmix does.
            "aresample=16000,aformat=channel_layouts=mono".to_string()
        } else {
            let sum: Vec<String> = chs.iter().map(|c| format!("c{c}")).collect();
            format!("pan=mono|c0={},aresample=16000", sum.join("+"))
        }
    };
    let split: String = (0..tracks.len()).map(|i| format!("[s{i}]")).collect();
    let mut graph = format!("{merged},asplit={}{split}", tracks.len());
    for (i, (_, chs)) in tracks.iter().enumerate() {
        graph.push_str(&format!(";[s{i}]{}[o{i}]", pan(chs)));
    }
    cmd.args(["-filter_complex", &graph]);

    let mut files = Vec::new();
    for (i, (stem, _)) in tracks.iter().enumerate() {
        let file = retained_audio::track_file(stem, format);
        cmd.args(["-map", &format!("[o{i}]")])
            .args(retained_audio::encoder_args(format))
            .arg(dir.join(&file));
        files.push(file);
    }
    let out = cmd.stdin(Stdio::null()).output()?;
    if !out.status.success() {
        return Err(ImportError::Decode(
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ));
    }
    Ok(files)
}

/// The meeting settings an import or retranscribe runs with, as a live
/// meeting started with `diarization` would.
pub fn meeting_config_for(config: &Config, diarization: Option<&str>) -> MeetingConfig {
    let d = &config.meeting.diarization;
    MeetingConfig {
        enabled: true,
        chunk_duration_secs: config.meeting.chunk_duration_secs,
        storage: StorageConfig {
            storage_path: if config.meeting.storage_path == "auto" {
                Config::data_dir().join("meetings")
            } else {
                PathBuf::from(&config.meeting.storage_path)
            },
            retain_audio: false,
            max_meetings: 0,
        },
        // Import writes its audio files itself, from the original.
        retain_audio: false,
        max_duration_mins: 0,
        vad_threshold: config.meeting.audio.vad_threshold,
        // An imported recording with both tracks gets the same echo guard
        // as a live meeting.
        echo_rules: (&config.meeting.audio).into(),
        diarization: d
            .enabled
            .then(|| crate::meeting::diarization::DiarizationConfig {
                enabled: true,
                backend: diarization.unwrap_or(&d.backend).to_string(),
                max_speakers: d.max_speakers,
                min_segment_ms: d.min_segment_ms,
                model_path: d.model_path.clone(),
                similarity_threshold: d.similarity_threshold,
                vad_window_secs: d.vad_window_secs,
                vad_hop_secs: d.vad_hop_secs,
                vad_rms_floor: d.vad_rms_floor,
            }),
    }
}

/// A one-track source has no microphone side, so hybrid's "mic is You,
/// cluster the rest" becomes plain clustering.
pub(crate) fn mono_diarization(config: &mut MeetingConfig) {
    if let Some(ref mut d) = config.diarization {
        if d.backend == "hybrid" {
            d.backend = "ml".to_string();
        }
    }
}

/// A meeting daemon for an import or retranscribe, and a task draining the
/// events nobody is listening to.
pub(crate) fn daemon_for(
    meeting_config: MeetingConfig,
    effective: &Config,
    resident: Option<Arc<dyn Transcriber>>,
) -> Result<MeetingDaemon, ImportError> {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<MeetingEvent>(256);
    tokio::spawn(async move { while rx.recv().await.is_some() {} });
    Ok(MeetingDaemon::new(meeting_config, effective, tx, resident)?)
}

/// What `meeting import` asks for.
#[derive(Debug, Clone, Default)]
pub struct ImportRequest {
    pub file: PathBuf,
    pub title: Option<String>,
    pub options: MeetingStartOptions,
    pub diarization: Option<String>,
    pub date: Option<DateTime<Utc>>,
    pub channels: Option<String>,
}

/// What `meeting import --json` prints at the end.
#[derive(Debug, Clone, Serialize)]
pub struct ImportOutcome {
    pub meeting_id: String,
    pub title: String,
    pub folder: PathBuf,
    pub started_at: DateTime<Utc>,
    pub duration_ms: u64,
    pub channels: String,
    pub segments: usize,
    pub engine: String,
    pub model: String,
    pub audio_files: Vec<String>,
}

/// Import `request.file` as a new meeting. `resident` is a transcriber to
/// use instead of loading one (tests pass a fake). `progress` is called
/// after every chunk and once at the end.
pub async fn import(
    config: &Config,
    request: &ImportRequest,
    resident: Option<Arc<dyn Transcriber>>,
    progress: &mut dyn FnMut(&Progress),
) -> Result<ImportOutcome, ImportError> {
    if request.file.is_dir() {
        if crate::meeting::meeting_recorder::manifest_in(&request.file).is_some() {
            return crate::meeting::meeting_recorder::import(config, request, progress).await;
        }
        return Err(ImportError::Unreadable {
            path: request.file.display().to_string(),
            reason: "it is a folder, and not one Omarchy Meeting Recorder wrote".to_string(),
        });
    }
    let ff = Ffmpeg::detect()?;
    if !request.file.is_file() {
        return Err(ImportError::Unreadable {
            path: request.file.display().to_string(),
            reason: "no such file".to_string(),
        });
    }
    let probed = probe(&ff, &request.file)?;
    let spec = request.channels.as_deref().unwrap_or("mono");
    let channels = Channels::parse(spec)?;
    channels.check(probed.channels, spec)?;

    let effective = crate::meeting::start::meeting_transcription_config(config, &request.options)
        .map_err(ImportError::Config)?;
    let mut meeting_config = meeting_config_for(config, request.diarization.as_deref());
    if channels == Channels::Mono {
        mono_diarization(&mut meeting_config);
    }
    let chunk_secs = meeting_config.chunk_duration_secs;
    let vad_threshold = meeting_config.vad_threshold;
    let storage = MeetingStorage::open(meeting_config.storage.clone())?;
    let mut daemon = daemon_for(meeting_config, &effective, resident)?;

    let title = request.title.clone().unwrap_or_else(|| {
        request
            .file
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Imported recording".to_string())
    });
    let started_at = request
        .date
        .or(probed.created)
        .or_else(|| {
            std::fs::metadata(&request.file)
                .and_then(|m| m.modified())
                .ok()
                .map(DateTime::<Utc>::from)
        })
        .unwrap_or_else(Utc::now);
    let id = daemon
        .start_at(
            Some(title.clone()),
            request.options.url.clone(),
            Some(started_at),
        )
        .await?;
    let folder = storage.get_meeting_path(&id)?;

    let source = Source {
        inputs: vec![request.file.clone()],
        channels: channels.clone(),
        duration_ms: probed.duration_ms,
    };
    // The audio first: if transcription fails, the meeting can still be
    // transcribed again from it.
    let audio_files = match keep_audio(&ff, &source, &folder) {
        Ok(files) => files,
        Err(e) => {
            tracing::warn!("Could not keep the imported audio: {}", e);
            Vec::new()
        }
    };

    let total = source.chunks_total(chunk_secs);
    let report = |state: &str, done: u32, position_ms: u64| Progress {
        meeting_id: id.to_string(),
        operation: "import".to_string(),
        state: state.to_string(),
        chunks_done: done,
        chunks_total: total.max(done),
        position_ms,
        duration_ms: source.duration_ms,
    };
    progress(&report("transcribing", 0, 0));
    let result = transcribe_source(
        &mut daemon,
        &ff,
        &source,
        chunk_secs,
        vad_threshold,
        &mut |done, position| progress(&report("transcribing", done, position)),
    )
    .await;
    // Stop either way, so a failed import keeps what was transcribed.
    let stopped = daemon.stop().await;
    let decoded_ms = match result {
        Ok(ms) => ms,
        Err(e) => {
            progress(&report("failed", 0, 0));
            return Err(e);
        }
    };
    stopped?;

    // Dated by the recording, not by how long transcribing took.
    if let Some(mut metadata) = storage.get_meeting(&id)? {
        metadata.ended_at = Some(started_at + chrono::Duration::milliseconds(decoded_ms as i64));
        metadata.duration_secs = Some(decoded_ms / 1000);
        storage.update_meeting(&metadata)?;
    }
    let segments = storage
        .load_transcript(&id)
        .map(|t| t.segments.len())
        .unwrap_or(0);
    let mut last = report("done", 0, decoded_ms);
    last.chunks_done = last.chunks_total;
    progress(&last);

    finish(&daemon, id).await;
    Ok(ImportOutcome {
        meeting_id: id.to_string(),
        title,
        folder,
        started_at,
        duration_ms: decoded_ms,
        channels: channels.describe(),
        segments,
        engine: format!("{:?}", effective.engine).to_lowercase(),
        model: effective.model_name().to_string(),
        audio_files,
    })
}

/// What a live meeting gets once it stops: the auto-summary when it is on,
/// then the manifest rewrite and `post_meeting_command`.
async fn finish(daemon: &MeetingDaemon, id: MeetingId) {
    if let Some(job) = daemon.auto_summary(id) {
        job.run(|event| tracing::debug!(?event, "Imported meeting summary"))
            .await;
    }
    daemon.post_meeting(id).run().await;
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::error::TranscribeError;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// Numbers each utterance it is given: "u0", "u1", ...
    pub(crate) struct CountingTranscriber(pub AtomicU32);

    impl Transcriber for CountingTranscriber {
        fn short_clips_are_cheap(&self) -> bool {
            true
        }

        fn transcribe(&self, samples: &[f32]) -> Result<String, TranscribeError> {
            if samples.iter().all(|s| s.abs() < 0.05) {
                return Ok(String::new());
            }
            Ok(format!("u{}", self.0.fetch_add(1, Ordering::SeqCst)))
        }
    }

    pub(crate) fn counting() -> Option<Arc<dyn Transcriber>> {
        Some(Arc::new(CountingTranscriber(AtomicU32::new(0))))
    }

    pub(crate) fn ffmpeg_present() -> bool {
        let found = Ffmpeg::detect().is_ok();
        if !found {
            eprintln!("skipping: no ffmpeg and ffprobe");
        }
        found
    }

    /// A WAV of `secs` seconds with a 440 Hz tone on each channel over its
    /// spans, at 16 kHz.
    pub(crate) fn wav(path: &Path, secs: f32, spans: &[&[(f32, f32)]]) {
        let spec = hound::WavSpec {
            channels: spans.len() as u16,
            sample_rate: SAMPLE_RATE,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(path, spec).unwrap();
        for i in 0..(secs * SAMPLE_RATE as f32) as usize {
            let t = i as f32 / SAMPLE_RATE as f32;
            for channel in spans {
                let on = channel.iter().any(|&(s, e)| t >= s && t < e);
                let v = if on {
                    0.5 * (2.0 * std::f32::consts::PI * 440.0 * t).sin()
                } else {
                    0.0
                };
                w.write_sample((v * 32767.0) as i16).unwrap();
            }
        }
        w.finalize().unwrap();
    }

    /// Config storing meetings in `dir`, with no summary and no hook.
    pub(crate) fn config_in(dir: &Path) -> Config {
        let mut config = Config::default();
        config.meeting.enabled = true;
        config.meeting.storage_path = dir.display().to_string();
        config.meeting.summary.auto = Some(false);
        config
    }

    #[test]
    fn channel_maps_parse_and_fit_the_file() {
        assert_eq!(Channels::parse("mono").unwrap(), Channels::Mono);
        assert_eq!(
            Channels::parse("mic,loopback").unwrap(),
            Channels::Mapped(vec![Side::Mic, Side::Loopback])
        );
        assert_eq!(
            Channels::parse("You, remote,-").unwrap(),
            Channels::Mapped(vec![Side::Mic, Side::Loopback, Side::Ignore])
        );
        assert!(Channels::parse("left,right").is_err());
        assert!(Channels::parse("ignore,-").is_err());
        let map = Channels::parse("mic,loopback").unwrap();
        assert!(map.check(2, "mic,loopback").is_ok());
        let err = map.check(1, "mic,loopback").unwrap_err().to_string();
        assert!(err.contains("has 1 channel, so name 1"), "{err}");
        assert!(Channels::Mono.check(6, "mono").is_ok());
        assert_eq!(map.describe(), "mic,loopback");
    }

    #[tokio::test]
    async fn a_stereo_call_imports_as_you_and_remote() {
        if !ffmpeg_present() {
            return;
        }
        let dir = tempfile::TempDir::new().unwrap();
        let file = dir.path().join("Client call.wav");
        wav(&file, 10.0, &[&[(1.0, 3.0)], &[(5.0, 7.0)]]);
        let config = config_in(&dir.path().join("meetings"));
        let date: DateTime<Utc> = "2026-11-02T15:00:00Z".parse().unwrap();
        let request = ImportRequest {
            file: file.clone(),
            date: Some(date),
            channels: Some("mic,loopback".to_string()),
            ..Default::default()
        };
        let mut seen = Vec::new();
        let outcome = import(&config, &request, counting(), &mut |p| seen.push(p.clone()))
            .await
            .unwrap();

        assert_eq!(outcome.title, "Client call");
        assert_eq!(outcome.channels, "mic,loopback");
        assert_eq!(outcome.started_at, date);
        assert!((outcome.duration_ms as i64 - 10_000).abs() < 50);
        assert_eq!(outcome.audio_files.len(), 3);

        let storage = MeetingStorage::open(meeting_config_for(&config, None).storage).unwrap();
        let id = MeetingId::parse(&outcome.meeting_id).unwrap();
        let data = storage.load_meeting_data(&id).unwrap();
        let lines: Vec<(String, u64)> = data
            .transcript
            .segments
            .iter()
            .map(|s| (s.speaker_display(), s.start_ms))
            .collect();
        assert_eq!(lines.len(), 2, "{lines:?}");
        // Each line starts at its channel's speech, give or take the
        // utterance splitter's padding.
        for ((speaker, start), (want, at)) in lines.iter().zip([("You", 1_000), ("Remote", 5_000)])
        {
            assert_eq!(speaker, want);
            assert!(start.abs_diff(at) < 400, "{lines:?}");
        }
        assert_eq!(data.metadata.started_at, date);
        assert_eq!(
            data.metadata.ended_at,
            Some(date + chrono::Duration::milliseconds(outcome.duration_ms as i64))
        );
        assert_eq!(data.metadata.duration_secs, Some(10));
        assert!(data.metadata.audio_retained);

        let m = crate::meeting::manifest::Manifest::read(&outcome.folder).unwrap();
        assert_eq!(m.status, "completed");
        let kinds: Vec<&str> = m.files.iter().map(|f| f.kind.as_str()).collect();
        for kind in ["transcript", "audio_mic", "audio_loopback", "audio_mixed"] {
            assert!(kinds.contains(&kind), "{kinds:?}");
        }
        for f in m.files.iter().filter(|f| f.kind.starts_with("audio_")) {
            let p = probe(&Ffmpeg::detect().unwrap(), &outcome.folder.join(&f.name)).unwrap();
            assert_eq!(p.channels, 1);
            assert!((p.duration_ms as i64 - 10_000).abs() < 100, "{f:?} {p:?}");
        }

        let first = seen.first().unwrap();
        assert_eq!(
            (first.operation.as_str(), first.state.as_str()),
            ("import", "transcribing")
        );
        let last = seen.last().unwrap();
        assert_eq!(last.state, "done");
        assert_eq!(last.chunks_done, last.chunks_total);
        assert_eq!(last.meeting_id, outcome.meeting_id);
    }

    #[tokio::test]
    async fn a_mono_recording_is_one_track_with_no_side() {
        if !ffmpeg_present() {
            return;
        }
        let dir = tempfile::TempDir::new().unwrap();
        let file = dir.path().join("memo.wav");
        // Stereo, but without --channels it is mixed down.
        wav(&file, 4.0, &[&[(0.5, 1.5)], &[(2.5, 3.5)]]);
        let config = config_in(&dir.path().join("meetings"));
        let request = ImportRequest {
            file,
            title: Some("Memo".to_string()),
            ..Default::default()
        };
        let outcome = import(&config, &request, counting(), &mut |_| {})
            .await
            .unwrap();
        assert_eq!(outcome.channels, "mono");
        assert_eq!(outcome.audio_files.len(), 1);
        assert!(outcome.audio_files[0].starts_with("mixed."));

        let storage = MeetingStorage::open(meeting_config_for(&config, None).storage).unwrap();
        let id = MeetingId::parse(&outcome.meeting_id).unwrap();
        let transcript = storage.load_transcript(&id).unwrap();
        assert_eq!(transcript.segments.len(), 2);
        assert!(transcript
            .segments
            .iter()
            .all(|s| s.source == AudioSource::Unknown && s.speaker_display() == "Unknown"));
    }

    #[tokio::test]
    async fn imports_refuse_what_they_cannot_read() {
        if !ffmpeg_present() {
            return;
        }
        let dir = tempfile::TempDir::new().unwrap();
        let config = config_in(&dir.path().join("meetings"));
        let missing = ImportRequest {
            file: dir.path().join("nope.mp3"),
            ..Default::default()
        };
        let err = import(&config, &missing, counting(), &mut |_| {})
            .await
            .unwrap_err();
        assert!(err.to_string().contains("no such file"), "{err}");

        let junk = dir.path().join("junk.mp3");
        std::fs::write(&junk, b"not audio").unwrap();
        let err = import(
            &config,
            &ImportRequest {
                file: junk,
                ..Default::default()
            },
            counting(),
            &mut |_| {},
        )
        .await
        .unwrap_err();
        assert!(err.to_string().starts_with("Cannot read"), "{err}");

        let mono = dir.path().join("mono.wav");
        wav(&mono, 1.0, &[&[(0.0, 1.0)]]);
        let err = import(
            &config,
            &ImportRequest {
                file: mono,
                channels: Some("mic,loopback".to_string()),
                ..Default::default()
            },
            counting(),
            &mut |_| {},
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("has 1 channel"), "{err}");
        // Nothing was half-created.
        let storage = MeetingStorage::open(meeting_config_for(&config, None).storage).unwrap();
        assert!(storage.list_meetings(None).unwrap().is_empty());
    }

    #[test]
    fn progress_round_trips_through_the_runtime_dir() {
        let runtime = tempfile::TempDir::new().unwrap();
        assert!(read_progress(runtime.path()).is_none());
        let p = Progress {
            meeting_id: "abc".to_string(),
            operation: "import".to_string(),
            state: "transcribing".to_string(),
            chunks_done: 2,
            chunks_total: 5,
            position_ms: 60_000,
            duration_ms: 150_000,
        };
        announce(runtime.path(), &p);
        assert_eq!(read_progress(runtime.path()), Some(p));
        assert!(runtime.path().join(PROGRESS_TRIGGER).exists());
    }
}
